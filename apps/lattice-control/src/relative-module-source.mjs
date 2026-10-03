import { createHash } from 'node:crypto';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { isExecutionDenied, openCircuitSummary } from './execution-recovery.mjs';
import { elicitationDenied } from './mcp-tool-elicitation.mjs';
import { failureEventDigest, rejectDiagnostic } from './relative-module-receipt.mjs';

const limits = Object.freeze({ sourceAgeMs: 300000, queryMs: 5000, failureBytes: 1048576 });
const id = value => typeof value === 'string' && /^[A-Za-z0-9._:-]{1,256}$/u.test(value);
const absolute = value => typeof value === 'string' && Buffer.byteLength(value) <= 4096
  && !/[\u0000-\u001f\u007f]/u.test(value) && path.isAbsolute(value);
const reject = reason => rejectDiagnostic(`SOURCE_${reason}`);

// No filesystem, subprocess, model, native lifecycle or durable mutation access.
// This projects existing owned execution authority, never a new repair grant.
export async function readOwnedDiagnosticSource(service, projectId, taskRef, selectors) {
  const names = ['claimId', 'threadId', 'turnId'];
  if (selectors && Object.hasOwn(selectors, 'failureItemId')) names.push('failureItemId');
  if (!id(projectId) || !/^[a-f0-9]{64}$/u.test(taskRef ?? '') || !selectors || Array.isArray(selectors)
      || Object.keys(selectors).sort().join(',') !== [...names].sort().join(',')
      || !names.every(key => id(selectors[key]))) reject('SELECTOR_REJECTED');
  const { claimId, threadId, turnId, failureItemId } = selectors;
  const codex = service.codex, started = Date.now(), deadline = started + limits.queryMs;
  const generation = codex.connectionGeneration, session = codex.appServerSessionId, requestSequence = codex.serverRequestSequence;
  const notices = () => {
    const rows = codex.notificationSnapshot?.({ threadId, turnId });
    if (!Array.isArray(rows)) reject('NOTIFICATION_MISSING');
    return rows;
  };
  const current = () => {
    const owner = service.owners.get(threadId);
    if (Date.now() < started || Date.now() >= deadline) reject('DEADLINE_EXCEEDED');
    if (service.closed || !codex.connected || codex.closePromise || codex.connectPromise
        || !Number.isSafeInteger(generation) || generation < 1
        || !/^app-server-session:sha256:[a-f0-9]{64}$/u.test(session ?? '')
        || codex.connectionGeneration !== generation || codex.appServerSessionId !== session
        || !codex.isTurnActive(threadId, turnId) || owner?.projectId !== projectId
        || owner.taskRef !== taskRef || owner.claimId !== claimId) reject('CURRENTNESS_REJECTED');
    if (codex.pendingServerRequestCount !== 0 || !Number.isSafeInteger(requestSequence) || requestSequence < 0
        || codex.serverRequestSequence !== requestSequence) reject('PERMISSION_PENDING_OR_UNKNOWN');
    if ((service.deniedTurns.get(`${threadId}:${turnId}`)?.size ?? 0) > 0
        || notices().some(row => row.message?.method === 'item/completed' && isExecutionDenied(row.message.params?.item))) reject('DENIED');
  };
  const claimFrom = detail => {
    const matches = detail?.claims?.filter(row => row.claim_id === claimId), claim = matches?.length === 1 ? matches[0] : null;
    if (detail?.source?.kind !== 'POSTGRESQL_CONTROL_PRODUCT' || detail.source.authority !== 'POSTGRESQL_TASK_LEDGER'
        || detail.id !== taskRef || detail.project_id !== projectId || detail.project?.id !== projectId
        || detail.project.active !== true || !id(detail.project.project_snapshot_id)
        || detail.status !== 'running' || detail.completion_verified !== false
        || !['SUBMITTED', 'RUNNING'].includes(detail.task?.ledger?.status)
        || detail.task.ledger.task_ref !== taskRef || detail.task.ledger.project_id !== projectId
        || detail.task.ledger.project_snapshot_id !== detail.project.project_snapshot_id
        || detail.task.ledger.blocker != null || detail.task.ledger.failure_code != null
        || !claim || claim.task_ref !== taskRef || claim.project_id !== projectId || claim.phase !== 'EXECUTION'
        || claim.thread_id !== threadId || claim.turn_id !== turnId || !id(claim.input_id) || !absolute(claim.worktree_path)
        || claim.archived !== false || claim.dispatch_started !== true || claim.turn_status !== 'TURN_BOUND'
        || !Number.isSafeInteger(claim.dispatch_sequence) || claim.dispatch_sequence < 1
        || !Number.isSafeInteger(claim.last_sequence) || claim.last_sequence <= claim.dispatch_sequence
        || !Array.isArray(claim.pending_inputs) || claim.pending_inputs.length
        || !Array.isArray(claim.pending_questions) || claim.pending_questions.length
        || !Array.isArray(detail.product?.observations) || elicitationDenied(detail, claim)) reject('CLAIM_REJECTED');
    if (detail.product.observations.some(row => row.claim_id === claimId && row.turn_id === turnId
        && row.summary === openCircuitSummary)) reject('CIRCUIT_OPEN');
    return claim;
  };
  const native = (thread, claim) => {
    const turns = thread.turns?.filter(row => row.id === turnId), turn = turns?.length === 1 ? turns[0] : null;
    const marker = `[LATTICE_TASK:${taskRef}:${claimId}:${claim.input_id}]`;
    const marked = item => item.type === 'userMessage' && item.content?.some(part => part.type === 'text'
      && (part.text === marker || part.text?.startsWith(`${marker}\n`)));
    if (thread.id !== threadId || thread.archived === true || !absolute(thread.cwd)
        || path.relative(claim.worktree_path, thread.cwd) !== '' || !turn || thread.turns.at(-1) !== turn
        || turn.status !== 'inProgress' || !Array.isArray(turn.items)
        || thread.turns.filter(row => row.items?.some(marked)).length !== 1 || turn.items.filter(marked).length !== 1
        || turn.items.some(isExecutionDenied)) reject('NATIVE_BINDING_REJECTED');
    const failures = turn.items.filter(item => failureItemId ? item.id === failureItemId
      : item.type === 'commandExecution' && item.status === 'failed' && Number.isSafeInteger(item.exitCode) && item.exitCode !== 0);
    if (!failureItemId && failures.length > 1) {
      try { reject('AMBIGUOUS_FAILURE'); } catch (error) {
        error.failureItemIds = [...new Set(failures.map(item => item.id).filter(id))].slice(0, 16);
        throw error;
      }
    }
    const selectedId = failures[0]?.id;
    if (!id(selectedId)) reject('FAILURE_MISSING');
    const events = notices().filter(row => row.message?.method === 'item/completed' && row.message.params?.item?.id === selectedId);
    if (failures.length !== 1 || events.length !== 1 || !Number.isSafeInteger(events[0].sequence) || events[0].sequence < 1
        || events[0].message.params.threadId !== threadId || events[0].message.params.turnId !== turnId
        || !isDeepStrictEqual(events[0].message.params.item, failures[0])) reject('NOTIFICATION_MISMATCH');
    const observedAt = Date.parse(events[0].observedAt), age = Date.now() - observedAt;
    if (!Number.isFinite(age) || age < 0 || age >= limits.sourceAgeMs) reject('EXPIRED');
    const failureJson = JSON.stringify(failures[0]);
    if (Buffer.byteLength(failureJson) >= limits.failureBytes) reject('TOO_LARGE');
    failureEventDigest(failures[0]); // Preserve existing preparation/receipt eligibility limits.
    return { failureJson, failureItemId: selectedId, event: events[0], validUntil: new Date(observedAt + limits.sourceAgeMs).toISOString() };
  };
  let timer;
  const expired = new Promise((_, fail) => { timer = setTimeout(() => {
    try { reject('DEADLINE_EXCEEDED'); } catch (error) { fail(error); }
  }, limits.queryMs); });
  const read = async action => { current(); const value = await Promise.race([action(), expired]); current(); return value; };
  try {
    // The same deadline covers queue wait and source reads. A timed-out lower
    // Runtime request is not cancelled; its late result cannot resume this query.
    return await Promise.race([service.serial(taskRef, async () => {
      current();
      const first = await read(() => service.store.detail(projectId, taskRef)), claim = claimFrom(first);
      const options = { effectIdentity: { expectedGeneration: generation, expectedSessionId: session } };
      const before = native(await read(() => codex.readThread(threadId, options)), claim);
      const last = await read(() => service.store.detail(projectId, taskRef)), latestClaim = claimFrom(last);
      if (!isDeepStrictEqual([first.project, first.task.ledger, first.ledger_head_digest, claim],
        [last.project, last.task.ledger, last.ledger_head_digest, latestClaim])) reject('CHANGED');
      const after = native(await read(() => codex.readThread(threadId, options)), latestClaim);
      if (!isDeepStrictEqual(before, after)) reject('CHANGED');
      const failureSha256 = createHash('sha256').update(after.failureJson).digest('hex');
      current();
      if (Date.now() >= Date.parse(after.validUntil)) reject('EXPIRED');
      return { schema: 'lattice.control.diagnostic-source.v1', projectRoot: latestClaim.worktree_path,
        binding: { projectId, taskRef, claimId, threadId, turnId, inputId: latestClaim.input_id, failureItemId: after.failureItemId },
        failureJson: after.failureJson, failureSha256,
        observedAt: after.event.observedAt, context: { projectId, taskId: taskRef, authority: 'authorized',
          projectScope: 'matched', lifecycle: 'active', freshness: 'current', circuitOpen: false, validUntil: after.validUntil },
        contextBasis: 'existing_owned_formal_execution', advisoryOnly: true, adopted: false,
        nativeSourceVerified: true, grantsNewAuthority: false, repairAuthorized: false,
        producerVerified: false, diagnosticSemanticsVerified: false, limits,
        lowerLevelReadCancelledOnTimeout: false };
    }), expired]);
  } finally { clearTimeout(timer); }
}
