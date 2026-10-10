import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { FormalTaskService } from '../src/formal-task-service.mjs';

const projectId = 'candidate-project', taskRef = 'a'.repeat(64), copy = value => structuredClone(value);
const prefix = 'LIFE_HARNESS_CANDIDATE_V1 ';
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
function fixture(t, lifeHarness = { enabled: true, origin: 'http://127.0.0.1:4317' }) {
  const workspace = path.resolve('candidate-memory-only'), hooks = {}, calls = { detail: 0, native: 0, updates: [], requests: [] };
  const claim = { project_id: projectId, task_ref: taskRef, claim_id: 'claim', phase: 'EXECUTION', thread_id: 'thread', turn_id: 'turn',
    input_id: 'input', worktree_path: workspace, archived: false, dispatch_started: true, dispatch_sequence: 2, last_sequence: 3,
    turn_status: 'TURN_BOUND', pending_inputs: [], pending_questions: [], mcp_permission: { version: 1, denied: false } };
  const detail = { id: taskRef, project_id: projectId, status: 'running', completion_verified: false, ledger_head_digest: 'b'.repeat(64),
    source: { kind: 'POSTGRESQL_CONTROL_PRODUCT', authority: 'POSTGRESQL_TASK_LEDGER' },
    project: { id: projectId, active: true, project_snapshot_id: 'snapshot' },
    task: { ledger: { task_ref: taskRef, project_id: projectId, project_snapshot_id: 'snapshot', status: 'SUBMITTED' } },
    claims: [claim], product: { observations: [] } };
  const importer = path.join(workspace, 'entry.mjs');
  const failure = { id: 'failure', type: 'commandExecution', status: 'failed', exitCode: 1,
    command: `node '${importer}'`, commandActions: [{ type: 'unknown', command: `node '${importer}'` }],
    aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(workspace, 'missing.mjs')}' imported from ${importer}\nDO_NOT_FORWARD_SECRET_OR_INSTRUCTIONS`,
    cwd: workspace, durationMs: 31 };
  const marker = { type: 'userMessage', content: [{ type: 'text', text: `[LATTICE_TASK:${taskRef}:claim:input]\noriginal task` }] };
  const turn = { id: 'turn', status: 'inProgress', items: [marker, failure] }, thread = { id: 'thread', cwd: workspace, turns: [turn] };
  const event = { sequence: 1, observedAt: new Date().toISOString(), message: { method: 'item/completed',
    params: { threadId: 'thread', turnId: 'turn', item: copy(failure) } } }, notices = [event];
  const codex = new EventEmitter(), observations = new Map();
  Object.assign(codex, { connected: true, connectionGeneration: 1, appServerSessionId: `app-server-session:sha256:${'f'.repeat(64)}`,
    pendingServerRequestCount: 0, serverRequestSequence: 0, closePromise: null, connectPromise: null,
    isTurnActive: (id, tid) => id === 'thread' && tid === 'turn' && turn.status === 'inProgress',
    notificationSnapshot: ({ threadId, turnId }) => copy(notices.filter(row => row.message.params.threadId === threadId && row.message.params.turnId === turnId)),
    async readThread(id, options) { assert.equal(id, 'thread'); assert.equal(options.effectIdentity.expectedGeneration, 1);
      calls.native++; await hooks.native?.(calls.native); return copy(thread); },
    async request(method, params, options) { calls.requests.push(copy({ method, params, options })); return await hooks.request?.(); },
    close: async () => {} });
  for (const name of ['connect', 'startThread', 'startTurn', 'resumeThread', 'interruptTurn', 'respond']) codex[name] = () => assert.fail(`forbidden ${name}`);
  const store = { async detail(p, ref) { assert.equal(p, projectId); assert.equal(ref, taskRef); calls.detail++; await hooks.detail?.(calls.detail); return copy(detail); },
    async update(input) {
      assert.equal(input.action, 'OBSERVE'); assert.equal(input.kind, 'PROGRESS'); calls.updates.push(copy(input));
      if (observations.has(input.request_id)) return { record: copy(observations.get(input.request_id)) };
      assert.equal(input.expected_sequence, claim.last_sequence);
      const record = { ...copy(input), sequence: ++claim.last_sequence };
      observations.set(input.request_id, record); detail.product.observations.push(record); detail.ledger_head_digest = String(claim.last_sequence).repeat(64);
      await hooks.update?.(input); return { record: copy(record) };
    }, close: async () => {} };
  const service = new FormalTaskService({ store, codex, lifeHarness, configurationLoader: () => assert.fail('configurationLoader') });
  service.owners.set('thread', { projectId, taskRef, claimId: 'claim' });
  const emit = (item = failure) => { event.message.params.item = copy(item); codex.emit('notification', copy(event.message)); };
  const settled = async () => { while (service.lifeHarnessCandidates.inflight.size) await Promise.all([...service.lifeHarnessCandidates.inflight]); };
  t.after(async () => { await settled(); await service.close(); });
  return { service, store, codex, claim, detail, failure, marker, turn, thread, event, notices, calls, hooks, emit, settled, observations };
}

test('owned real source guards lead to one persisted intent and same-turn steer, with no execution or raw log', async t => {
  const s = fixture(t); s.emit(); s.emit(); await s.settled();
  assert.equal(s.calls.native, 2); assert.equal(s.calls.detail, 4); assert.equal(s.calls.updates.length, 1); assert.equal(s.calls.requests.length, 1);
  const request = s.calls.requests[0]; assert.equal(request.method, 'turn/steer'); assert.equal(request.params.expectedTurnId, 'turn');
  assert.equal(request.params.threadId, 'thread'); assert.equal(request.options.expectedGeneration, 1);
  assert.equal(request.options.expectedSessionId, s.codex.appServerSessionId); assert.ok(request.options.timeoutMs > 0 && request.options.timeoutMs <= 5000);
  const text = request.params.input[0].text;
  assert.match(text, /node --experimental-vm-modules --disable-warning=ExperimentalWarning '[^'\r\n]+life-harness-client\.mjs' prepare --origin 'http:\/\/127\.0\.0\.1:4317'/u);
  assert.match(text, /--project 'candidate-project' --task '[a-f0-9]{64}' --claim 'claim' --thread 'thread' --turn 'turn' --failure 'failure'/u);
  assert.match(text, /readbackCommand/u); assert.doesNotMatch(text, /LATTICE_TASK:|DO_NOT_FORWARD|missing\.mjs|entry\.mjs/u);
  const summary = s.calls.updates[0].summary; assert.ok(summary.length <= 240);
  assert.deepEqual(JSON.parse(summary.slice(prefix.length)), { state: 'steer_intent', reason: 'NODE_RELATIVE_MODULE_CANDIDATE', failureIdHash: hash('failure'), advisoryOnly: true, adopted: false });
  assert.equal(s.failure.exitCode, 1);
  // A same-turn steer becomes another user message, but never duplicates the
  // dispatch marker required by both source and diagnostic receipt validation.
  s.turn.items.push({ type: 'userMessage', content: [{ type: 'text', text }] });
  assert.equal((await s.service.diagnosticSource(projectId, taskRef, { claimId: 'claim', threadId: 'thread', turnId: 'turn', failureItemId: 'failure' })).nativeSourceVerified, true);
});

test('disabled configuration has zero automatic source, persistence and native effects', async t => {
  for (const config of [{}, { enabled: false, origin: 'https://example.test' }]) {
    const s = fixture(t, config); s.emit(); await s.settled();
    assert.deepEqual(s.calls, { detail: 0, native: 0, updates: [], requests: [] });
  }
});

test('only a trusted exact loopback origin enables hints; delayed listen origin is captured once', async t => {
  for (const origin of ['http://localhost:4317', 'http://[::1]:4317', 'https://127.0.0.1', 'http://example.test', 'http://user@127.0.0.1', 'http://127.0.0.1/path', 'http://127.0.0.1/?x=1']) {
    assert.throws(() => new FormalTaskService({ store: {}, codex: new EventEmitter(), lifeHarness: { enabled: true, origin } }), /loopback/u);
  }
  let origin, resolutions = 0;
  const s = fixture(t, { enabled: true, origin: () => { resolutions++; return origin; } });
  s.emit(); await s.settled(); assert.equal(s.calls.detail, 0);
  origin = 'http://127.0.0.1:54321'; s.emit(); origin = 'http://example.test'; await s.settled();
  assert.equal(resolutions, 2); assert.equal(s.calls.requests.length, 1);
  assert.match(s.calls.requests[0].params.input[0].text, /127\.0\.0\.1:54321/u);
});

test('other failures produce bounded typed skips, while one eligible candidate remains available', async t => {
  const s = fixture(t);
  for (let n = 0; n < 20; n++) {
    const item = { ...s.failure, id: `other-${n}`, aggregatedOutput: 'unrelated command failed' };
    s.emit(item); s.emit(item);
  }
  await s.settled(); assert.equal(s.calls.native, 0); assert.equal(s.calls.requests.length, 0); assert.equal(s.calls.updates.length, 3);
  assert.ok(s.calls.updates.every(row => JSON.parse(row.summary.slice(prefix.length)).state === 'skipped'));
  s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 1);
  s.emit({ ...s.failure, id: 'helper-failure' }); await s.settled(); assert.equal(s.calls.requests.length, 1);
  assert.equal(s.service.lifeHarnessCandidates.turns.values().next().value.ids.size, 4);
});

test('unsupported command/package, ambiguous error lines and external paths never steer', async t => {
  for (const [name, change] of [
    ['command', s => { s.failure.commandActions[0].command = 'echo node'; }],
    ['malformed actions', s => { s.failure.commandActions = {}; }],
    ['package', s => { s.failure.aggregatedOutput = 'Error [ERR_MODULE_NOT_FOUND]: Cannot find package x'; }],
    ['two errors', s => { s.failure.aggregatedOutput += `\n${s.failure.aggregatedOutput}`; }],
    ['outside', s => { s.failure.aggregatedOutput = s.failure.aggregatedOutput.replace(path.join(s.claim.worktree_path, 'missing.mjs'), path.resolve('outside.mjs')); }],
  ]) await t.test(name, async t => { const s = fixture(t); change(s); s.emit(); await s.settled();
    assert.equal(s.calls.requests.length, 0); assert.equal(s.calls.updates.length, 1); assert.equal(JSON.parse(s.calls.updates[0].summary.slice(prefix.length)).state, 'skipped'); });
});

test('unowned, inactive, verification, pending, denied and expired events cannot cause native hints', async t => {
  for (const [name, change] of [
    ['unowned', s => s.service.owners.clear()], ['ended', s => { s.turn.status = 'completed'; }],
    ['verification', s => { s.claim.phase = 'VERIFICATION'; }], ['pending input', s => s.claim.pending_inputs.push({})],
    ['pending question', s => s.claim.pending_questions.push({})], ['native pending', s => { s.codex.pendingServerRequestCount = 1; }],
    ['unknown permission', s => { delete s.claim.mcp_permission; }], ['denied permission', s => { s.claim.mcp_permission.denied = true; }],
    ['denied turn', s => s.service.deniedTurns.set('thread:turn', new Set(['denied']))],
    ['expired', s => { s.event.observedAt = new Date(Date.now() - 300001).toISOString(); }],
    ['missing original marker', s => s.turn.items.shift()], ['wrong notification', s => { s.event.message.params.threadId = 'foreign'; }],
  ]) await t.test(name, async t => { const s = fixture(t); change(s); s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 0); });
});

test('post-source and post-intent changes prevent steering without repairing or restarting', async t => {
  const changes = [s => { s.codex.connectionGeneration++; }, s => { s.codex.appServerSessionId = `app-server-session:sha256:${'e'.repeat(64)}`; },
    s => { s.codex.serverRequestSequence++; }, s => { s.codex.pendingServerRequestCount++; }, s => s.service.owners.clear(),
    s => { s.turn.status = 'completed'; }, s => { s.claim.input_id = 'next-input'; }, s => { s.claim.turn_id = 'next-turn'; },
    s => s.claim.pending_questions.push({}), s => s.claim.pending_inputs.push({}), s => { s.claim.mcp_permission.denied = true; },
    s => s.service.deniedTurns.set('thread:turn', new Set(['denied'])), s => { s.detail.completion_verified = true; }];
  for (const timing of ['source', 'intent']) for (const [index, change] of changes.entries()) await t.test(`${timing} ${index}`, async t => {
    const s = fixture(t);
    if (timing === 'source') s.hooks.detail = n => { if (n === 3) change(s); }; else s.hooks.update = () => change(s);
    s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 0);
  });
});

test('failed durable write sends nothing; uncertain native ACK is attempted once even after state recreation and clipped preview', async t => {
  const failed = fixture(t); failed.hooks.update = () => { throw new Error('store unavailable'); };
  failed.emit(); await failed.settled(); assert.equal(failed.calls.requests.length, 0);
  const s = fixture(t); s.hooks.request = () => { throw Object.assign(new Error('timeout after send'), { code: 'TIMEOUT' }); };
  s.emit(); s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 1);
  s.detail.product.observations = []; s.service.lifeHarnessCandidates.turns.clear();
  s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 1);
  assert.equal(s.calls.updates.length, 2); assert.equal(s.observations.size, 1);
});

test('source queue is not nested and close during a held fresh read prevents any later effects', async t => {
  const s = fixture(t); let release, entered;
  const began = new Promise(resolve => { entered = resolve; });
  s.hooks.detail = n => n === 1 ? new Promise(resolve => { release = resolve; entered(); }) : undefined;
  s.emit(); await began; s.service.closed = true; release(); await s.settled();
  assert.equal(s.calls.requests.length, 0); assert.equal(s.calls.updates.length, 0);
  s.service.closed = false;
});

test('expiry or clock reversal after persisted intent sends no stale hint', async t => {
  for (const shift of [5001, -1000]) await t.test(String(shift), async t => {
    const s = fixture(t); let clock = Date.now();
    t.mock.method(Date, 'now', () => clock);
    s.hooks.update = () => { clock += shift; };
    s.emit(); await s.settled(); assert.equal(s.calls.updates.length, 1); assert.equal(s.calls.requests.length, 0);
  });
});

test('bounded active turn table does not evict a prior live dedupe record', async t => {
  const s = fixture(t); s.codex.isTurnActive = () => true;
  for (let n = 0; n < 256; n++) s.service.lifeHarnessCandidates.turns.set(`held-${n}`, { threadId: `thread-${n}`, turnId: `turn-${n}` });
  s.emit(); await s.settled(); assert.equal(s.service.lifeHarnessCandidates.turns.size, 256);
  assert.equal(s.calls.detail, 0); assert.equal(s.calls.requests.length, 0);
});

test('unsafe trusted helper paths cannot produce shell text', async t => {
  for (const character of ["'", '"', '\u2018', '\u201d', '$', '`', '!', '^', '\n']) await t.test(JSON.stringify(character), async t => {
    const s = fixture(t); s.service.lifeHarnessCandidates.client = path.resolve(`unsafe${character}path`, 'life-harness-client.mjs');
    s.emit(); await s.settled(); assert.equal(s.calls.requests.length, 0);
  });
});

test('the original event classifier cannot substitute for the verified source item', async t => {
  const s = fixture(t), forged = copy(s.failure);
  s.failure.aggregatedOutput = 'ordinary non-module failure'; s.event.message.params.item = copy(s.failure);
  s.codex.emit('notification', { method: 'item/completed', params: { threadId: 'thread', turnId: 'turn', item: forged } });
  await s.settled(); assert.equal(s.calls.requests.length, 0);
  assert.equal(JSON.parse(s.calls.updates[0].summary.slice(prefix.length)).reason, 'SOURCE_CHANGED');
});
