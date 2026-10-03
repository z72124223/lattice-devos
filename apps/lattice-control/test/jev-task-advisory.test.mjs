import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { FormalTaskService } from '../src/formal-task-service.mjs';
import { openCircuitSummary } from '../src/execution-recovery.mjs';
import { ENDPOINT, MODEL, MAX_RESPONSE_BYTES } from '../src/jev-choice-protocol.mjs';

const projectId = 'private-project', taskRef = 'a'.repeat(64);
const clone = value => structuredClone(value);
const names = ['git-resolution', 'codex-resolution', 'none_applicable', 'insufficient_information'];
const responseBody = (choice = 'git-resolution') => ({ model: MODEL, usage: { input_tokens: 10, output_tokens: 5 },
  answers: { diagnosis: { type: 'choice', choice, confidence: 0.9,
    probabilities: Object.fromEntries(names.map(name => [name, name === choice ? 1 : 0])) } } });
const response = body => new Response(typeof body === 'string' ? body : JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } });
const realFetch = globalThis.fetch;
let networkAttempts = 0;
test.before(() => { globalThis.fetch = () => { networkAttempts++; assert.fail('real external network forbidden'); }; });
test.after(() => { globalThis.fetch = realFetch; assert.equal(networkAttempts, 0, 'exceptions swallowed by product code cannot hide transport attempts'); });

function fixture(t, config = { enabled: true }) {
  const effects = [], sent = [], calls = { detail: 0, read: 0 }, hooks = {};
  const forbidden = name => () => { effects.push(name); assert.fail(name); };
  const claim = { project_id: projectId, task_ref: taskRef, claim_id: 'claim', phase: 'EXECUTION', thread_id: 'thread',
    turn_id: 'turn', input_id: 'input', dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND',
    archived: false, last_sequence: 3, pending_questions: [], pending_inputs: [], mcp_permission: { version: 1, denied: false } };
  const detail = { source: { kind: 'POSTGRESQL_CONTROL_PRODUCT', authority: 'POSTGRESQL_TASK_LEDGER' },
    id: taskRef, project_id: projectId, status: 'running', completion_verified: false, ledger_head_digest: 'b'.repeat(64),
    project: { id: projectId, active: true, project_snapshot_id: 'snapshot' }, claims: [claim], product: { observations: [] },
    task: { ledger: { status: 'SUBMITTED', task_ref: taskRef, project_id: projectId, project_snapshot_id: 'snapshot', blocker: null, failure_code: null } } };
  const failure = { id: 'failure', type: 'commandExecution', status: 'failed', exitCode: 1,
    command: `${JSON.stringify('C:\\Program Files\\PowerShell\\7\\pwsh.exe')} -Command ${JSON.stringify('node C:/private-source/resolve.mjs')}`,
    aggregatedOutput: 'PRIVATE_RAW_LOG\nError: spawn git ENOENT\n', extraRaw: 'NEVER_SEND_ME' };
  const marker = { type: 'userMessage', content: [{ type: 'text', text: `[LATTICE_TASK:${taskRef}:claim:input]\nprivate prompt` }] };
  const turn = { id: 'turn', status: 'inProgress', items: [marker, failure] };
  // Actual saved native Thread has no `archived` property. Archive authority is
  // read from the durable claim, not a test-only false flag on this response.
  const thread = { id: 'thread', turns: [turn] };
  const notification = { sequence: 4, observedAt: new Date().toISOString(),
    message: { method: 'item/completed', params: { threadId: 'thread', turnId: 'turn', item: clone(failure) } } };
  const history = [notification];
  const codex = new EventEmitter();
  Object.assign(codex, { connected: true, connectionGeneration: 1, appServerSessionId: `app-server-session:sha256:${'f'.repeat(64)}`,
    pendingServerRequestCount: 0, serverRequestSequence: 0, closePromise: null, connectPromise: null,
    isTurnActive: (id, tid) => id === 'thread' && tid === 'turn' && turn.status === 'inProgress',
    notificationSnapshot: ({ method = null, threadId = null, turnId = null } = {}) => clone(history.filter(({ message }) => {
      const tid = message.params?.threadId ?? message.params?.thread?.id ?? null;
      const turn = message.params?.turnId ?? message.params?.turn?.id ?? null;
      return (method === null || message.method === method) && (threadId === null || tid === threadId) && (turnId === null || turn === turnId);
    })),
    async readThread(id, options) {
      assert.equal(id, 'thread');
      assert.deepEqual(options.effectIdentity, { expectedGeneration: 1, expectedSessionId: `app-server-session:sha256:${'f'.repeat(64)}` });
      calls.read++; await hooks.read?.(calls.read); return clone(thread);
    }, close: async () => {} });
  const store = { async detail(p, task) {
    assert.equal(p, projectId); assert.equal(task, taskRef); calls.detail++; await hooks.detail?.(calls.detail); return clone(detail);
  }, update: forbidden('store.update') };
  for (const name of ['connect', 'startThread', 'startTurn', 'resumeThread', 'resumeEmptyThread', 'interruptTurn', 'respond', 'request']) codex[name] = forbidden(name);
  const transport = async (url, init) => {
    assert.equal(url, ENDPOINT); assert.equal(init.redirect, 'error');
    sent.push({ body: JSON.parse(init.body), init });
    return hooks.transport ? hooks.transport(url, init) : response(responseBody());
  };
  const service = new FormalTaskService({ store, codex, configurationLoader: forbidden('configurationLoader'), jevAdvisory: { transport, ...config } });
  service.owners.set('thread', { projectId, taskRef, claimId: 'claim' });
  const request = { intent: 'read-only-advice', claimId: 'claim', threadId: 'thread', turnId: 'turn', failureItemId: 'failure' };
  const call = () => service.jevAdvisory(projectId, taskRef, request);
  const sync = () => { notification.message.params.item = clone(failure); };
  t.after(async () => { await service.close(); assert.deepEqual(effects, []); });
  return { claim, detail, failure, marker, turn, thread, notification, history, codex, store, service, request, calls, hooks, sent, call, sync };
}

function limited(result) {
  assert.equal(result.advisory_only, true); assert.equal(result.adopted, false);
  assert.equal(result.authorization_verified, false); assert.equal(result.diagnostic_semantics_verified, false);
  assert.equal(result.mode, 'simulated');
  assert.ok(!JSON.stringify(result).includes('PRIVATE'));
}

test('default disabled and explicit invocation required: no reads or transport', async t => {
  for (const config of [{}, { enabled: false }, { enabled: true }]) {
    const s = fixture(t, config);
    if (config.enabled) delete s.request.intent;
    const result = await s.call(); limited(result);
    assert.equal(result.reason, config.enabled ? 'explicit_invocation_required' : 'disabled');
    assert.deepEqual(s.calls, { detail: 0, read: 0 }); assert.equal(s.sent.length, 0);
  }
});

test('product method reads live-source interfaces twice and sends only closed classifications', async t => {
  const s = fixture(t), result = await s.call(); limited(result);
  assert.equal(result.decision, 'selected'); assert.equal(result.native_source_verified, true);
  assert.equal(result.selected_procedure, 'git-resolution'); assert.equal(result.usage.simulated, true);
  assert.deepEqual(s.calls, { detail: 2, read: 2 }); assert.equal(s.sent.length, 1);
  assert.deepEqual(s.sent[0].body.state, { schema: 'lattice.jev-failure-projection.v1', platform: 'win32', category: 'tool_launch',
    failure: 'git_executable_unavailable', failed: true });
  assert.deepEqual(Object.keys(s.sent[0].body), ['model', 'state', 'questions']);
  for (const raw of ['command', 'aggregatedOutput', 'PRIVATE', 'NEVER_SEND_ME', 'private-project', taskRef, 'private-source', 'private prompt']) {
    assert.ok(!JSON.stringify(s.sent[0].body).includes(raw), raw);
  }
});

test('Windows Codex resolution is the second existing purpose', async t => {
  const s = fixture(t); s.failure.aggregatedOutput = 'Error: Codex runtime was not found; install Codex or set codexBin to an exact trusted path\n'; s.sync();
  s.hooks.transport = () => response(responseBody('codex-resolution'));
  const result = await s.call(); limited(result); assert.equal(result.selected_procedure, 'codex-resolution');
  assert.equal(s.sent[0].body.state.failure, 'codex_executable_unavailable');
});

const negatives = [
  ['caller auth flag', s => { s.request.authorized = true; }],
  ['caller projection', s => { s.request.state = { failed: true }; }],
  ['caller command', s => { s.request.command = 'git'; }],
  ['caller source hash', s => { s.request.sourceSha256 = 'a'.repeat(64); }],
  ['wrong project', s => { s.detail.project.id = 'other'; }],
  ['unknown authority', s => { s.detail.source.authority = 'CALLER'; }],
  ['snapshot mismatch', s => { s.detail.task.ledger.project_snapshot_id = 'older'; }],
  ['completed', s => { s.detail.completion_verified = true; }],
  ['inactive project', s => { s.detail.project.active = false; }],
  ['claim missing', s => { s.detail.claims = []; }],
  ['duplicate claim', s => { s.detail.claims.push(clone(s.claim)); }],
  ['wrong input', s => { s.claim.input_id = 'other'; }],
  ['wrong phase', s => { s.claim.phase = 'VERIFICATION'; }],
  ['archived claim', s => { s.claim.archived = true; }],
  ['interrupted', s => { s.claim.turn_status = 'INTERRUPTED'; }],
  ['unknown durable permission', s => { delete s.claim.mcp_permission; }],
  ['unknown permission version', s => { s.claim.mcp_permission.version = 2; }],
  ['durable denial', s => { s.claim.mcp_permission.denied = true; }],
  ['pending question', s => { s.claim.pending_questions.push({}); }],
  ['queued input', s => { s.claim.pending_inputs.push({}); }],
  ['circuit open', s => { s.detail.product.observations.push({ claim_id: 'claim', turn_id: 'turn', summary: openCircuitSummary }); }],
  ['native pending request', s => { s.codex.pendingServerRequestCount = 1; }],
  ['unknown pending count', s => { delete s.codex.pendingServerRequestCount; }],
  ['connection closing', s => { s.codex.closePromise = Promise.resolve(); }],
  ['connection starting', s => { s.codex.connectPromise = Promise.resolve(); }],
  ['disconnected', s => { s.codex.connected = false; }],
  ['owner missing', s => { s.service.owners.clear(); }],
  ['native marker missing', s => { s.turn.items.shift(); }],
  ['wrong native identity', s => { s.thread.id = 'other'; }],
  ['native turn completed', s => { s.turn.status = 'completed'; }],
  ['archived thread', s => { s.thread.archived = true; }],
  ['duplicate failure', s => { s.turn.items.push(clone(s.failure)); }],
  ['notification evicted', s => { s.history.length = 0; }],
  ['duplicate notification', s => { s.history.push(clone(s.notification)); }],
  ['notification differs', s => { s.notification.message.params.item.exitCode = 2; }],
  ['expired', s => { s.notification.observedAt = new Date(Date.now() - 300001).toISOString(); }],
  ['future', s => { s.notification.observedAt = new Date(Date.now() + 10000).toISOString(); }],
  ['unknown time', s => { s.notification.observedAt = 'unknown'; }],
  ['successful item', s => { s.failure.exitCode = 0; s.sync(); }],
  ['agent text', s => { s.failure.type = 'agentMessage'; s.sync(); }],
  ['raw oversized', s => { s.failure.aggregatedOutput = 'x'.repeat(65537); s.sync(); }],
  ['missing output', s => { s.failure.aggregatedOutput = ''; s.sync(); }],
  ['ambiguous output', s => { s.failure.aggregatedOutput += 'Error: spawn codex ENOENT\n'; s.sync(); }],
  ['relative module none applicable', s => { s.failure.aggregatedOutput = 'Error [ERR_MODULE_NOT_FOUND]: missing ./message.mjs'; s.sync(); }],
  ['unknown platform', s => { s.failure.command = 'sh -c codex'; s.sync(); }],
  ['native denied', s => { s.failure.status = 'declined'; s.sync(); }],
  ['policy denial', s => { s.failure.aggregatedOutput += 'CreateProcess rejected: blocked by policy'; s.sync(); }],
  ['retained denial', s => { s.service.deniedTurns.set('thread:turn', new Set(['denied'])); }],
];
test('closed selectors, authority, source and state gates all stop with zero transport', async t => {
  for (const [name, edit] of negatives) await t.test(name, async child => {
    const s = fixture(child); edit(s); const result = await s.call(); limited(result);
    assert.equal(result.decision, 'abstain'); assert.equal(s.sent.length, 0, name);
  });
});

test('permission and source changes during asynchronous reads stop before transmission', async t => {
  for (const [name, edit] of [
    ['new native pending request', s => { s.codex.pendingServerRequestCount = 1; }],
    ['request arrived then settled', s => { s.codex.serverRequestSequence++; }],
    ['generation', s => { s.codex.connectionGeneration++; }],
    ['session', s => { s.codex.appServerSessionId = `app-server-session:sha256:${'e'.repeat(64)}`; }],
    ['new denial before durable callback', s => { s.history.push({ message: { method: 'item/completed', params: {
      threadId: 'thread', turnId: 'turn', item: { id: 'denial', type: 'mcpToolCall', status: 'failed', error: { message: 'user rejected tool call' } } } } }); }],
    ['changed failure', s => { s.failure.aggregatedOutput += ' changed'; s.sync(); }],
  ]) await t.test(name, async child => {
    const s = fixture(child); s.hooks.read = n => { if (n === 2) edit(s); };
    limited(await s.call()); assert.equal(s.sent.length, 0);
  });
  const s = fixture(t); s.hooks.detail = n => { if (n === 2) s.claim.last_sequence++; };
  assert.equal((await s.call()).reason, 'source_changed'); assert.equal(s.sent.length, 0);
});

test('wrong purpose, low confidence, malformed/oversized/version responses: one request, no effects', async t => {
  const low = responseBody(); low.answers.diagnosis.confidence = 0.79;
  const wrongModel = responseBody(); wrongModel.model = 'jev-next';
  const extra = responseBody(); extra.command = 'PRIVATE EXECUTE';
  for (const [body, reason] of [[responseBody('codex-resolution'), 'choice_evidence_missing'], [low, 'low_confidence'],
    [wrongModel, 'invalid_response'], [extra, 'invalid_response'], ['{', 'invalid_response'],
    [' '.repeat(MAX_RESPONSE_BYTES + 1), 'response_too_large']]) {
    const s = fixture(t); s.hooks.transport = () => response(body);
    const result = await s.call(); limited(result); assert.equal(result.reason, reason); assert.equal(s.sent.length, 1);
  }
});

test('lifecycle changes while awaiting the response invalidate the advice', async t => {
  const s = fixture(t); s.hooks.transport = () => { s.codex.serverRequestSequence++; return response(responseBody()); };
  const result = await s.call(); limited(result); assert.equal(result.reason, 'permission_pending_or_unknown'); assert.equal(s.sent.length, 1);
});

test('unknown source exceptions are not returned as private text', async t => {
  const s = fixture(t); s.hooks.detail = () => { throw Object.assign(new Error('PRIVATE source error'), { reason: 'PRIVATE', httpStatus: 'PRIVATE' }); };
  const result = await s.call(); limited(result); assert.equal(result.reason, 'source_unavailable'); assert.equal(s.sent.length, 0);
});

test('disabled mode never reads a key, and enabled live mode requires one without transmitting', async t => {
  let keyReads = 0;
  const env = { get TYPESAFE_API_KEY() { keyReads++; return undefined; } };
  const disabled = fixture(t, { transport: undefined, env });
  assert.equal((await disabled.call()).reason, 'disabled'); assert.equal(keyReads, 0);
  const enabled = fixture(t, { enabled: true, transport: undefined, env });
  const result = await enabled.call();
  assert.equal(result.reason, 'api_key_missing_or_invalid'); assert.equal(keyReads, 1);
  assert.equal(result.http_status, null); assert.equal(enabled.sent.length, 0);
});

test('last synchronous transport guard catches a permission change after the second native read', async t => {
  let s;
  const env = { get TYPESAFE_API_KEY() { s.codex.serverRequestSequence++; return 'test-only-key'; } };
  s = fixture(t, { enabled: true, transport: undefined, env });
  const result = await s.call();
  assert.equal(result.decision, 'abstain'); assert.equal(result.reason, 'transport_failed');
  assert.equal(result.http_status, null); assert.equal(s.sent.length, 0);
  assert.equal(networkAttempts, 0, 'direct fetch must be counted outside the caught transport error');
});

test('another thread or turn notification cannot supply the selected native source', async t => {
  for (const field of ['threadId', 'turnId']) {
    const s = fixture(t); s.notification.message.params[field] = 'other';
    const result = await s.call(); limited(result);
    assert.equal(result.reason, 'native_source_missing'); assert.equal(s.sent.length, 0);
  }
});

test('unrelated denial notifications do not override this owned turn', async t => {
  const s = fixture(t);
  for (const [threadId, turnId] of [['other', 'turn'], ['thread', 'other']]) {
    s.history.push({ message: { method: 'item/completed', params: { threadId, turnId,
      item: { id: 'foreign-denial', type: 'mcpToolCall', status: 'failed', error: { message: 'user rejected tool call' } } } } });
  }
  const result = await s.call(); limited(result); assert.equal(result.decision, 'selected'); assert.equal(s.sent.length, 1);
});

test('product timeout aborts once with no retry', async t => {
  const s = fixture(t); s.hooks.transport = () => new Promise(() => {});
  const result = await s.call(); limited(result); assert.equal(result.reason, 'deadline_exceeded');
  assert.equal(s.sent.length, 1); assert.equal(s.sent[0].init.signal.aborted, true);
});

test('slow source read releases task serial within the same deadline and ignores late results', async t => {
  const s = fixture(t); let finishRead;
  s.hooks.detail = () => new Promise(resolve => { finishRead = resolve; });
  const started = performance.now();
  const pending = s.call();
  const queued = s.service.serial(taskRef, () => 'next explicit action can proceed');
  const result = await pending; limited(result);
  assert.equal(result.reason, 'deadline_exceeded');
  assert.ok(performance.now() - started < 6500, 'must not wait for the Runtime default 60 second timeout');
  assert.equal(await queued, 'next explicit action can proceed');
  assert.deepEqual(s.calls, { detail: 1, read: 0 }); assert.equal(s.sent.length, 0);
  finishRead(); await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(s.calls, { detail: 1, read: 0 }); assert.equal(s.sent.length, 0);
});
