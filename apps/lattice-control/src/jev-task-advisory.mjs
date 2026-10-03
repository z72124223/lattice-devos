import { isDeepStrictEqual } from 'node:util';
import { isExecutionDenied, openCircuitSummary } from './execution-recovery.mjs';
import { elicitationDenied } from './mcp-tool-elicitation.mjs';
import { MODEL, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, exchangeJevChoice } from './jev-choice-protocol.mjs';

const keys = (value, names) => value !== null && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name));
const id = value => typeof value === 'string' && /^[A-Za-z0-9._:-]{1,256}$/u.test(value);
const reject = reason => { throw Object.assign(new Error(reason), { advisoryReason: reason }); };
const safeReasons = new Set(['failure_evidence_missing', 'insufficient_information', 'platform_unknown', 'none_applicable',
  'stale_or_unknown', 'permission_pending_or_unknown', 'denied', 'identity_or_state_unknown', 'denied_or_unknown',
  'circuit_open', 'native_binding_unknown', 'native_source_missing', 'expired_or_unknown', 'source_changed',
  'request_too_large', 'api_key_missing_or_invalid', 'invalid_configuration', 'invalid_response', 'redirect_rejected',
  'http_error', 'unsupported_encoding', 'response_too_large', 'truncated_response', 'deadline_exceeded', 'transport_failed']);
const limits = Object.freeze({ source_age_ms: 300000, deadline_ms: 5000,
  local_request_bytes: MAX_REQUEST_BYTES, local_response_bytes: MAX_RESPONSE_BYTES });
const criteria = Object.freeze({
  'git-resolution': 'Git executable lookup failed. Read-only inspection of the existing trusted Git resolver; never execute or install.',
  'codex-resolution': 'Windows Codex executable lookup failed. Read-only inspection of the existing Desktop/npm resolver; never execute or install.',
  none_applicable: 'Neither executable resolution procedure applies.',
  insufficient_information: 'Evidence does not identify an applicable procedure.',
});

// Only closed classifications leave this module. Native output remains untrusted
// evidence: matching text is not a verified causal diagnosis or repair authority.
function projectFailure(item) {
  if (item?.type !== 'commandExecution' || item.status !== 'failed'
      || !Number.isSafeInteger(item.exitCode) || item.exitCode === 0
      || typeof item.command !== 'string' || Buffer.byteLength(item.command) > 16384
      || typeof item.aggregatedOutput !== 'string' || Buffer.byteLength(item.aggregatedOutput) > 65536) reject('failure_evidence_missing');
  const output = item.aggregatedOutput;
  if (!output.trim() || /no diagnostic output/iu.test(output)) reject('insufficient_information');
  // This first product slice requires a native Windows PowerShell command envelope.
  // Do not infer a Windows execution platform just from the Control host's OS.
  const prefix = /^("[^"]+") -Command /u.exec(item.command);
  let shell;
  try { shell = prefix && JSON.parse(prefix[1]); } catch { /* unknown envelope */ }
  if (typeof shell !== 'string' || !/^[A-Za-z]:\\(?:[^\\\r\n]+\\)*(?:pwsh|powershell)\.exe$/iu.test(shell)) reject('platform_unknown');
  const git = /(?:^|\n)(?:Error: )?spawn git ENOENT(?:\r?$)/mu.test(output)
    || /(?:^|\n)(?:ProjectInspectionError: )?a trusted absolute Git executable could not be resolved(?:\r?$)/mu.test(output)
    || /(?:^|\n)\s*\+\s*FullyQualifiedErrorId\s*:\s*CommandNotFoundException(?:\r?$)/mu.test(output)
      && /(?:^|\n)git\s*:/mu.test(output);
  const codex = /(?:^|\n)(?:Error: )?spawn codex ENOENT(?:\r?$)/mu.test(output)
    || /(?:^|\n)Error: Codex runtime was not found; install Codex or set codexBin to an exact trusted path(?:\r?$)/mu.test(output);
  if (git && codex) reject('insufficient_information');
  if (!git && !codex) reject('none_applicable');
  return Object.freeze({ schema: 'lattice.jev-failure-projection.v1', platform: 'win32',
    category: 'tool_launch', failure: git ? 'git_executable_unavailable' : 'codex_executable_unavailable',
    failed: true });
}

// Called only by the explicitly invoked FormalTaskService method. `service` and
// `config` are trusted startup dependencies, never request-supplied provenance.
export async function adviseOwnedTaskWithJev(service, projectId, taskRef, request, config) {
  const result = { schema: 'lattice.control.jev-advisory.v1', model: MODEL,
    mode: typeof config.transport === 'function' ? 'simulated' : 'live', decision: 'abstain',
    reason: null, selected_procedure: null, advisory_only: true, adopted: false,
    authorization_verified: false, diagnostic_semantics_verified: false,
    native_source_verified: false, confidence: null, usage: null, http_status: null, limits };
  const stop = reason => ({ ...result, reason });
  if (config.enabled !== true) return stop('disabled');
  if (request?.intent !== 'read-only-advice') return stop('explicit_invocation_required');
  if (!keys(request, ['intent', 'claimId', 'threadId', 'turnId', 'failureItemId'])
      || !id(projectId) || !/^[a-f0-9]{64}$/u.test(taskRef ?? '')
      || !['claimId', 'threadId', 'turnId', 'failureItemId'].every(key => id(request[key]))) return stop('invalid_request');
  const { claimId, threadId, turnId, failureItemId } = request;
  const started = Date.now(), codex = service.codex;
  const generation = codex.connectionGeneration, session = codex.appServerSessionId, requestSequence = codex.serverRequestSequence;
  const notices = () => codex.notificationSnapshot({ threadId, turnId });
  const current = () => {
    const owner = service.owners.get(threadId);
    if (service.closed || !codex.connected || codex.closePromise || codex.connectPromise || !Number.isSafeInteger(generation) || generation < 1
        || !/^app-server-session:sha256:[a-f0-9]{64}$/u.test(session ?? '')
        || codex.connectionGeneration !== generation || codex.appServerSessionId !== session
        || !codex.isTurnActive(threadId, turnId) || Date.now() - started < 0 || Date.now() - started > limits.deadline_ms
        || owner?.projectId !== projectId || owner.taskRef !== taskRef || owner.claimId !== claimId) reject('stale_or_unknown');
    // Native pending requests precede the durable callback queued on serial().
    // Requiring the actual pending count also closes that permission race.
    if (codex.pendingServerRequestCount !== 0 || !Number.isSafeInteger(requestSequence) || requestSequence < 0
        || codex.serverRequestSequence !== requestSequence) reject('permission_pending_or_unknown');
    if ((service.deniedTurns.get(`${threadId}:${turnId}`)?.size ?? 0) > 0
        || notices().some(row => row.message.method === 'item/completed' && isExecutionDenied(row.message.params?.item))) reject('denied');
  };
  const claimFrom = detail => {
    const matches = detail.claims?.filter(row => row.claim_id === claimId);
    const claim = matches?.length === 1 ? matches[0] : null;
    if (detail.source?.kind !== 'POSTGRESQL_CONTROL_PRODUCT' || detail.source?.authority !== 'POSTGRESQL_TASK_LEDGER'
        || detail.id !== taskRef || detail.project_id !== projectId || detail.project?.id !== projectId
        || detail.project.active !== true || !id(detail.project.project_snapshot_id)
        || detail.completion_verified !== false || detail.status !== 'running'
        || !['SUBMITTED', 'RUNNING'].includes(detail.task?.ledger?.status)
        || detail.task.ledger.task_ref !== taskRef || detail.task.ledger.project_id !== projectId
        || detail.task.ledger.project_snapshot_id !== detail.project.project_snapshot_id
        || detail.task.ledger.blocker != null || detail.task.ledger.failure_code != null
        || !claim || claim.task_ref !== taskRef || claim.project_id !== projectId || claim.phase !== 'EXECUTION'
        || claim.thread_id !== threadId || claim.turn_id !== turnId || !id(claim.input_id)
        || claim.archived !== false || claim.dispatch_started !== true || claim.turn_status !== 'TURN_BOUND'
        || !Number.isSafeInteger(claim.dispatch_sequence) || claim.dispatch_sequence < 1
        || !Number.isSafeInteger(claim.last_sequence) || claim.last_sequence <= claim.dispatch_sequence
        || !Array.isArray(claim.pending_inputs) || claim.pending_inputs.length
        || !Array.isArray(claim.pending_questions) || claim.pending_questions.length
        || !Array.isArray(detail.product?.observations)) reject('identity_or_state_unknown');
    if (elicitationDenied(detail, claim)) reject('denied_or_unknown');
    if (detail.product.observations.some(row => row.claim_id === claimId && row.turn_id === turnId
        && row.summary === openCircuitSummary)) reject('circuit_open');
    return claim;
  };
  const observedFailure = (thread, claim) => {
    const turns = thread.turns?.filter(row => row.id === turnId), turn = turns?.length === 1 ? turns[0] : null;
    const marker = `[LATTICE_TASK:${taskRef}:${claimId}:${claim.input_id}]`;
    const matchesMarker = item => item.type === 'userMessage' && item.content?.some(part =>
      part.type === 'text' && (part.text === marker || part.text?.startsWith(`${marker}\n`)));
    // Native Thread does not require an archived field. Durable claim.archived
    // plus the connected owner's active turn establish eligibility; reject an
    // explicit native archive indication without inventing a missing flag.
    if (thread.id !== threadId || thread.archived === true || !turn || thread.turns.at(-1) !== turn
        || turn.status !== 'inProgress' || !Array.isArray(turn.items)
        || thread.turns.filter(row => row.items?.some(matchesMarker)).length !== 1
        || turn.items.filter(matchesMarker).length !== 1) reject('native_binding_unknown');
    if (turn.items.some(isExecutionDenied)) reject('denied');
    const items = turn.items.filter(item => item.id === failureItemId);
    const events = notices().filter(row => row.message.method === 'item/completed' && row.message.params?.item?.id === failureItemId);
    if (items.length !== 1 || events.length !== 1 || !isDeepStrictEqual(events[0].message.params.item, items[0])
        || !Number.isSafeInteger(events[0].sequence) || events[0].sequence < 1) reject('native_source_missing');
    const age = Date.now() - Date.parse(events[0].observedAt);
    if (!Number.isFinite(age) || age < 0 || age > limits.source_age_ms) reject('expired_or_unknown');
    return { item: items[0], event: events[0] };
  };
  const remaining = () => Math.max(1, limits.deadline_ms - (Date.now() - started));
  const readWithinWindow = async read => {
    current();
    let timer;
    try {
      // Release the task serial queue at this deadline, even when a lower-layer
      // readonly request has a longer timeout. Its late result is not resumed.
      return await Promise.race([read(), new Promise((_, fail) => {
        timer = setTimeout(() => fail(Object.assign(new Error('deadline_exceeded'), { advisoryReason: 'deadline_exceeded' })), remaining());
      })]);
    } finally { clearTimeout(timer); }
  };
  try {
    current();
    const first = await readWithinWindow(() => service.store.detail(projectId, taskRef)); current();
    const claim = claimFrom(first);
    const options = { effectIdentity: { expectedGeneration: generation, expectedSessionId: session } };
    const thread = await readWithinWindow(() => codex.readThread(threadId, options)); current();
    const before = observedFailure(thread, claim);
    const projection = projectFailure(before.item);
    const last = await readWithinWindow(() => service.store.detail(projectId, taskRef)); current();
    const latestClaim = claimFrom(last);
    if (!isDeepStrictEqual([first.project, first.ledger_head_digest, claim], [last.project, last.ledger_head_digest, latestClaim])) reject('source_changed');
    const latest = await readWithinWindow(() => codex.readThread(threadId, options)); current();
    if (!isDeepStrictEqual(before, observedFailure(latest, latestClaim))) reject('source_changed');
    result.native_source_verified = true;
    // Construct each outgoing field explicitly. Neither raw request fields nor
    // a provider's arbitrary strings can be spread into this body.
    const body = JSON.stringify({ model: MODEL, state: { schema: projection.schema, platform: projection.platform,
      category: projection.category, failure: projection.failure, failed: projection.failed },
    questions: { diagnosis: { type: 'choice', instructions: 'Choose only an applicable read-only diagnosis. No execution or authorization.', criteria } } });
    if (Buffer.byteLength(body) > MAX_REQUEST_BYTES) reject('request_too_large');
    const headers = { 'Content-Type': 'application/json', Accept: 'application/json', 'Accept-Encoding': 'identity' };
    if (config.transport === undefined) {
      const key = (config.env ?? process.env).TYPESAFE_API_KEY;
      if (typeof key !== 'string' || !/^[\x21-\x7e]{1,4096}$/u.test(key)) reject('api_key_missing_or_invalid');
      headers.Authorization = `Bearer ${key}`;
    } else if (typeof config.transport !== 'function') reject('invalid_configuration');
    const transport = (url, init) => {
      current(); // Last synchronous check, immediately before any transmission.
      observedFailure(latest, latestClaim);
      return (config.transport ?? globalThis.fetch)(url, init);
    };
    const response = await exchangeJevChoice(body, Object.keys(criteria), { headers, transport, deadlineMs: remaining() });
    result.http_status = response.httpStatus;
    current(); observedFailure(latest, latestClaim);
    // A response cannot repair, grant authority, or select an unrelated resolver.
    const { answer, usage } = response;
    result.confidence = answer.confidence;
    result.usage = { ...usage, simulated: Boolean(config.transport), source: config.transport ? 'simulated' : 'server_reported' };
    if (answer.confidence < 0.8) return stop('low_confidence');
    if (['none_applicable', 'insufficient_information'].includes(answer.choice)) return stop(answer.choice);
    const expected = projection.failure === 'git_executable_unavailable' ? 'git-resolution' : 'codex-resolution';
    if (answer.choice !== expected) return stop('choice_evidence_missing');
    return { ...result, decision: 'selected', selected_procedure: answer.choice };
  } catch (error) {
    if (Number.isInteger(error?.httpStatus) && error.httpStatus >= 100 && error.httpStatus <= 599) result.http_status = error.httpStatus;
    const reason = error?.advisoryReason ?? error?.reason;
    return stop(safeReasons.has(reason) ? reason : 'source_unavailable');
  }
}
