import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import fsPromises from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { FormalTaskService } from '../src/formal-task-service.mjs';
import { recoveryPrompt, openCircuitSummary } from '../src/execution-recovery.mjs';
const projectId = 'memory-project';
const taskRef = 'a'.repeat(64);
const workspace = path.resolve('formal-test-memory-workspace');
const clone = (value) => structuredClone(value);
// Runtime hashes semantic OBSERVE fields; expected_sequence is only a freshness guard.
const semanticCommand = ({ expected_sequence, ...value }) => value;
const nativeMarker = (claim, inputId = claim.input_id) => `[LATTICE_TASK:${taskRef}:${claim.claim_id}:${inputId}]`;
function claim(phase = 'EXECUTION', fields = {}) {
  return { task_ref: taskRef, claim_id: phase.toLowerCase() + '-claim', phase,
    model: 'gpt-6-astra', worktree_path: workspace, prompt: 'saved work prompt',
    created_at: '2026-09-05T00:00:00Z', thread_id: phase.toLowerCase() + '-thread',
    turn_id: null, input_id: null, last_sequence: 1, dispatch_started: false,
    dispatch_sequence: null, execution_sequence: null, turn_status: null,
    archived: false, pending_inputs: [], pending_questions: [], ...fields };
}
function nativeTurn(owner, id, status = 'inProgress', output = null, inputId = owner.input_id) {
  return { id, status, items: [
    { type: 'userMessage', content: [{ type: 'text', text: nativeMarker(owner, inputId) + '\nretained input' }] },
    ...(output ? [{ type: 'agentMessage', text: JSON.stringify(output) }] : []),
  ] };
}

// This stub models the persisted boundary, especially request-id replay: an
// already committed request must preserve its semantic command; sequence may refresh.
class MemoryStore {
  constructor(claims) {
    this.state = { id: taskRef, project_id: projectId, objective: 'memory-only work',
      success_criteria: 'meaningful acceptance', completion_verified: false,
      ledger_head_digest: 'b'.repeat(64), task: { client_request_id: 'original-client' },
      claims: clone(claims), product: { observations: [] } };
    this.calls = [];
    this.requests = new Map();
  }
  async detail(project, task) {
    assert.equal(project, projectId); assert.equal(task, taskRef);
    return clone(this.state);
  }
  async update(command) {
    this.calls.push(clone(command));
    assert.equal(command.action, 'OBSERVE', 'this scenario must preserve existing claims');
    const retained = this.requests.get(command.request_id);
    if (retained) {
      if (JSON.stringify(semanticCommand(retained.command)) !== JSON.stringify(semanticCommand(command))) {
        throw Object.assign(new Error('retained request was changed'), { code: 'CONTROL_PRODUCT_IDEMPOTENCY_CONFLICT' });
      }
      return { record: clone(retained.record) };
    }
    const current = this.state.claims.find((row) => row.claim_id === command.claim_id);
    assert.ok(current);
    assert.equal(command.expected_sequence, current.last_sequence, 'observation must use current sequence');
    const record = { ...clone(command), sequence: current.last_sequence + 1 };
    this.requests.set(command.request_id, { command: clone(command), record: clone(record) });
    this.state.product.observations.push(record);
    current.last_sequence = record.sequence;
    if (command.kind === 'THREAD_BOUND') current.thread_id = command.thread_id;
    if (command.kind === 'REOPENED') current.archived = false;
    if (command.kind === 'INPUT_QUEUED') current.pending_inputs.push(record);
    if (command.kind === 'DISPATCH_STARTED') {
      Object.assign(current, { dispatch_started: true, dispatch_sequence: record.sequence,
        turn_id: null, input_id: command.input_id, turn_status: 'DISPATCH_STARTED' });
      current.pending_inputs = current.pending_inputs.filter((row) => row.input_id !== command.input_id);
      if (current.phase === 'VERIFICATION') current.execution_sequence = this.state.claims.find((row) => row.phase === 'EXECUTION').dispatch_sequence;
    }
    if (command.kind === 'TURN_BOUND') Object.assign(current, { turn_id: command.turn_id, turn_status: 'TURN_BOUND' });
    if (['TURN_COMPLETED', 'TURN_FAILED', 'INTERRUPTED', 'CLAIM_FAILED'].includes(command.kind)) current.turn_status = command.kind;
    if (['QUESTION_REQUESTED', 'APPROVAL_REQUESTED'].includes(command.kind)) current.pending_questions.push(record);
    if (['QUESTION_RESOLVED', 'APPROVAL_RESOLVED'].includes(command.kind)) current.pending_questions = current.pending_questions.filter((row) => row.approval_id !== command.approval_id);
    return { record: clone(record) };
  }
  invalidate() {}
}
class MemoryCodex extends EventEmitter {
  constructor(threads = []) { super(); this.threads = new Map(threads.map((row) => [row.id, clone(row)])); this.calls = []; }
  async startThread() { assert.fail('must not create another native thread'); }
  async listThreads() { assert.fail('the saved bound thread must be used directly'); }
  async readThread(id) { this.calls.push({ method: 'readThread', id }); assert.ok(this.threads.has(id)); return clone(this.threads.get(id)); }
  isTurnActive(id, turnId) { return this.threads.get(id)?.turns.some((row) => row.id === turnId && row.status === 'inProgress') ?? false; }
  async resumeEmptyThread(id) {
    this.calls.push({ method: 'resumeEmptyThread', id });
    const thread = this.threads.get(id); assert.deepEqual(thread.turns, []); return clone(thread);
  }
  async resumeThread(id, options) {
    this.calls.push({ method: 'resumeThread', id, options: clone(options) });
    const thread = this.threads.get(id); assert.equal(thread.turns.at(-1)?.id, options.expectedTurnId); return clone(thread);
  }
  async startTurn(id, text, options) {
    this.calls.push({ method: 'startTurn', id, text, options: clone(options) });
    const turn = { id: `new-native-turn-${this.calls.filter((row) => row.method === 'startTurn').length}`, status: 'inProgress',
      items: [{ type: 'userMessage', content: [{ type: 'text', text }] }] };
    this.threads.get(id).turns.push(turn); return clone(turn);
  }
  deferServerRequest(id) { this.calls.push({ method: 'deferServerRequest', id }); }
  rejectServerRequest(id) { this.calls.push({ method: 'rejectServerRequest', id }); }
  respond(id, response) { this.calls.push({ method: 'respond', id, response: clone(response) }); }
  async request(method, params) { this.calls.push({ method, ...clone(params) }); return {}; }
  async interruptTurn(id, turnId) {
    this.calls.push({ method: 'interruptTurn', id, turnId });
    const turn = this.threads.get(id).turns.find((row) => row.id === turnId);
    turn.status = 'interrupted'; return clone(turn);
  }
  async close() {}
}
function setup(t, claims, threads) {
  const store = new MemoryStore(claims), codex = new MemoryCodex(threads);
  const service = new FormalTaskService({ store, codex, configurationLoader: async () => assert.fail('must not launch a real importer') });
  t.after(() => service.close());
  return { store, codex, service };
}

function recoveryFixture(t) {
  const executor = claim('EXECUTION', { turn_id: 'recovery-turn', input_id: 'recovery-input',
    last_sequence: 3, dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const fixture = setup(t, [executor], [{ id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id)] }]);
  const owner = { projectId, taskRef, claimId: executor.claim_id };
  fixture.service.owners.set(executor.thread_id, owner);
  const deny = async (id, service = fixture.service) => {
    const item = { type: 'commandExecution', id, status: 'failed', aggregatedOutput: 'CreateProcess rejected: blocked by policy' };
    const turn = fixture.codex.threads.get(executor.thread_id).turns[0];
    if (!turn.items.some((row) => row.id === id)) turn.items.push(item);
    await service.redirectDeniedTurn(owner, { turnId: executor.turn_id, item });
  };
  return { ...fixture, executor, owner, deny };
}

test('first policy denial redirects the same native turn to safer alternatives exactly once', async (t) => {
  const { codex, store, deny } = recoveryFixture(t);
  await deny('denial-one'); await deny('denial-one');
  const redirects = codex.calls.filter((row) => row.method === 'turn/steer');
  assert.equal(redirects.length, 1);
  assert.equal(redirects[0].expectedTurnId, 'recovery-turn');
  assert.equal(redirects[0].input[0].text, recoveryPrompt);
  assert.equal(codex.calls.some((row) => ['interruptTurn', 'startTurn', 'startThread'].includes(row.method)), false);
  assert.equal(store.state.product.observations.length, 1);
  assert.equal(store.state.completion_verified, false);
});

test('a second policy denial interrupts and cannot automatically dispatch queued input or verify', async (t) => {
  const { service, store, codex, deny } = recoveryFixture(t);
  await deny('denial-one'); await deny('denial-two'); await deny('denial-two');
  store.state.claims[0].pending_inputs.push({ input_id: 'queued', summary: 'retry' });
  service.beginVerification = async () => assert.fail('open circuit must not verify');
  await service.reconcile(projectId, taskRef, { advance: true, resumePrepared: true });
  assert.equal(codex.calls.filter((row) => row.method === 'interruptTurn').length, 1);
  assert.equal(codex.calls.some((row) => row.method === 'startTurn'), false);
  assert.equal(store.state.claims[0].turn_status, 'INTERRUPTED');
  assert.equal(store.state.product.observations.at(-1).summary, openCircuitSummary);
});

test('native denial history reconstructs the open circuit after restart despite a completed final reply', async (t) => {
  const { service, store, codex, executor } = recoveryFixture(t);
  const turn = codex.threads.get(executor.thread_id).turns[0];
  turn.status = 'completed';
  turn.items.push(...['one', 'two'].map((id) => ({ id, type: 'commandExecution', status: 'declined' })));
  service.beginVerification = async () => assert.fail('denied work must not be promoted to completion');
  await service.reconcile(projectId, taskRef, { advance: true });
  await service.reconcile(projectId, taskRef, { advance: true });
  assert.equal(store.state.claims[0].turn_status, 'TURN_COMPLETED');
  assert.equal(store.state.completion_verified, false);
  assert.equal(store.state.product.observations.length, 1);
  assert.equal(codex.calls.some((row) => row.method === 'startTurn'), false);
});

test('a lost redirect response is never replayed, including after the progress preview is truncated', async (t) => {
  const { store, codex, deny } = recoveryFixture(t);
  codex.request = async () => { throw new Error('response lost'); };
  await assert.rejects(deny('denial-one'), /response lost/);
  // Runtime retains the idempotency record even when the bounded snapshot
  // no longer includes this progress observation.
  store.state.product.observations = [];
  store.state.claims[0].last_sequence += 10;
  const restarted = new FormalTaskService({ store, codex });
  t.after(() => restarted.close());
  let repeated = 0; codex.request = async () => { repeated += 1; };
  await deny('denial-one', restarted);
  assert.equal(repeated, 0);
});

test('a stale denial notification cannot redirect another turn', async (t) => {
  const { service, owner, codex, store } = recoveryFixture(t);
  await service.redirectDeniedTurn(owner, { turnId: 'older-turn', item: { id: 'old', type: 'commandExecution', status: 'declined' } });
  assert.equal(codex.calls.length, 0); assert.equal(store.calls.length, 0);
});

test('one denied action followed by a successful alternative still reaches independent verification', async (t) => {
  const { service, codex, executor, deny } = recoveryFixture(t);
  await deny('denial-one');
  codex.threads.get(executor.thread_id).turns[0].status = 'completed';
  let verified = 0; service.beginVerification = async () => { verified += 1; };
  await service.reconcile(projectId, taskRef, { advance: true });
  assert.equal(verified, 1);
});

test('the native notification route redirects failed tool output while retaining exact task ownership', async (t) => {
  const { service, codex, store, executor } = recoveryFixture(t);
  codex.emit('notification', { method: 'item/completed', params: {
    threadId: executor.thread_id, turnId: executor.turn_id,
    item: { id: 'native-error', type: 'mcpToolCall', status: 'failed', error: { message: 'blocked by policy' } },
  } });
  await Promise.all([...service.operations.values()]);
  assert.equal(codex.calls.filter((row) => row.method === 'turn/steer').length, 1);
  assert.equal(store.state.product.observations[0].claim_id, executor.claim_id);
});

test('durable observation failure prevents an unrecorded redirect or automatic retry', async (t) => {
  const { store, codex, deny } = recoveryFixture(t);
  store.update = async () => { throw new Error('database unavailable'); };
  await assert.rejects(deny('denial-one'), /database unavailable/);
  await deny('denial-one');
  assert.equal(codex.calls.some((row) => row.method === 'turn/steer'), false);
});

test('a denied turn whose binding response was lost is reconciled before the circuit is recorded', async (t) => {
  const executor = claim('EXECUTION', { input_id: 'lost-binding', last_sequence: 2, dispatch_started: true,
    dispatch_sequence: 2, turn_status: 'DISPATCH_STARTED' });
  const turn = nativeTurn(executor, 'lost-native-turn', 'completed');
  turn.items.push(...['one', 'two'].map((id) => ({ id, type: 'commandExecution', status: 'declined' })));
  const { store, service, codex } = setup(t, [executor], [{ id: executor.thread_id, turns: [turn] }]);
  await service.reconcile(projectId, taskRef, { advance: true });
  assert.equal(store.state.claims[0].turn_id, turn.id);
  assert.equal(store.state.product.observations.at(-1).summary, openCircuitSummary);
  assert.equal(codex.calls.some((row) => row.method === 'startTurn'), false);
});

test('an explicitly revised executor result can replace a verifier from an older blocked generation', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'revised-result', input_id: 'new-plan', last_sequence: 8,
    dispatch_started: true, dispatch_sequence: 6, turn_status: 'TURN_COMPLETED' });
  const verifier = claim('VERIFICATION', { turn_id: 'old-verification', input_id: 'old-input', last_sequence: 5,
    dispatch_started: true, dispatch_sequence: 2, execution_sequence: 2, turn_status: 'INTERRUPTED' });
  const old = nativeTurn(verifier, verifier.turn_id, 'interrupted');
  old.items.push(...['one', 'two'].map((id) => ({ id, type: 'commandExecution', status: 'declined' })));
  const { service } = setup(t, [executor, verifier], [
    { id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed')] },
    { id: verifier.thread_id, turns: [old] },
  ]);
  let replacements = 0; service.beginVerification = async () => { replacements += 1; };
  await service.reconcile(projectId, taskRef, { advance: true });
  assert.equal(replacements, 1);
});

test('saved bound empty thread: explicit start resumes the original and dispatches once', async (t) => {
  const executor = claim();
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [] }]);
  await service.start(projectId, taskRef);
  await service.start(projectId, taskRef);
  assert.equal(codex.calls.filter((row) => row.method === 'resumeEmptyThread').length, 1);
  const sent = codex.calls.filter((row) => row.method === 'startTurn');
  assert.equal(sent.length, 1); assert.equal(sent[0].id, executor.thread_id);
  assert.ok(sent[0].text.startsWith(nativeMarker(executor, executor.claim_id) + '\n'));
  assert.deepEqual(store.calls.map((row) => row.kind), ['DISPATCH_STARTED', 'TURN_BOUND']);
  assert.equal(store.state.claims[0].thread_id, executor.thread_id);
});

test('unacknowledged dispatch: exact marker recovers the retained native turn without resending', async (t) => {
  const executor = claim('EXECUTION', { input_id: 'retained-input', last_sequence: 2, dispatch_started: true,
    dispatch_sequence: 2, turn_status: 'DISPATCH_STARTED' });
  const wrong = nativeTurn(executor, 'unrelated-native-turn', 'completed', null, 'other-input');
  const right = nativeTurn(executor, 'retained-native-turn');
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [wrong, right] }]);
  await service.start(projectId, taskRef);
  await service.start(projectId, taskRef);
  assert.equal(store.state.claims[0].turn_id, right.id);
  assert.deepEqual(store.calls.map((row) => row.kind), ['TURN_BOUND']);
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 0);
  assert.equal(codex.calls.filter((row) => row.method === 'resumeEmptyThread').length, 0);
});

test('new executor generation dispatches a new turn on the same verifier instead of finishing the old verdict', { concurrency: false }, async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'execution-turn-2', input_id: 'execution-input-2',
    last_sequence: 9, dispatch_started: true, dispatch_sequence: 7, turn_status: 'TURN_COMPLETED' });
  const verifier = claim('VERIFICATION', { turn_id: 'verification-turn-1', input_id: 'verification-input-1',
    last_sequence: 5, dispatch_started: true, dispatch_sequence: 2, execution_sequence: 2, turn_status: 'TURN_COMPLETED' });
  const { store, codex, service } = setup(t, [executor, verifier], [
    { id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed', { summary: 'new implementation', artifact_path: 'artifact.mjs', test_path: 'artifact.test.mjs' })] },
    { id: verifier.thread_id, turns: [nativeTurn(verifier, verifier.turn_id, 'completed', { passed: true, summary: 'old generation only' })] },
  ]);
  // Keep the production beginVerification/dispatch methods intact. Only the
  // filesystem boundary is virtualized; no files, Git worktrees or Codex run.
  const files = new Map([[path.join(workspace, 'artifact.mjs'), Buffer.from('new artifact')], [path.join(workspace, 'artifact.test.mjs'), Buffer.from('meaningful test')]]);
  t.mock.method(fsPromises, 'realpath', async (value) => {
    const target = path.resolve(String(value)); assert.ok(target === workspace || files.has(target)); return target;
  });
  t.mock.method(fsPromises, 'readFile', async (value) => { const bytes = files.get(String(value)); assert.ok(bytes); return bytes; });
  t.mock.method(fsPromises, 'stat', async (value) => ({ isFile: () => files.has(String(value)) }));
  syncBuiltinESMExports();
  t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports(); });
  service.finishVerification = async () => assert.fail('old verifier must not be finalized');
  await service.reconcile(projectId, taskRef, { advance: true });
  const sent = codex.calls.filter((row) => row.method === 'startTurn');
  assert.equal(sent.length, 1); assert.equal(sent[0].id, verifier.thread_id);
  assert.ok(sent[0].text.startsWith(nativeMarker(verifier, 'verify:execution-input-2') + '\n'));
  assert.equal(sent[0].options.outputSchema.properties.passed.type, 'boolean');
  assert.deepEqual(store.calls.map((row) => row.kind), ['INPUT_QUEUED', 'DISPATCH_STARTED', 'TURN_BOUND']);
  assert.equal(store.state.claims.length, 2);
  const retained = store.state.claims.find((row) => row.phase === 'VERIFICATION');
  assert.equal(retained.claim_id, verifier.claim_id); assert.equal(retained.execution_sequence, 7);
  assert.notEqual(retained.turn_id, verifier.turn_id);
});

test('question from an old native turn is rejected before any durable question is recorded', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'current-turn', input_id: 'current-input', last_sequence: 3,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const { store, codex, service } = setup(t, [executor], []);
  service.owners.set(executor.thread_id, { projectId, taskRef, claimId: executor.claim_id });
  codex.emit('serverRequest', { id: 41, method: 'item/tool/requestUserInput',
    params: { threadId: executor.thread_id, turnId: 'old-turn', questions: [] } });
  await Promise.allSettled([...service.operations.values()]);
  await Promise.resolve();
  assert.deepEqual(codex.calls.filter((row) => row.method === 'rejectServerRequest').map((row) => row.id), [41]);
  assert.equal(store.calls.length, 0); assert.equal(service.questions.size, 0);
});

test('native response failure retries the same resolution request identity and then delivers once', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'current-turn', input_id: 'current-input', last_sequence: 3,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const { store, codex, service } = setup(t, [executor], []);
  const owner = { projectId, taskRef, claimId: executor.claim_id };
  await service.recordQuestion(owner, { id: 42, method: 'item/tool/requestUserInput',
    params: { threadId: executor.thread_id, turnId: executor.turn_id, questions: [{ id: 'choice', question: 'Which option?' }] } });
  const questionId = [...service.questions.keys()][0];
  let attempts = 0, delivered = 0;
  codex.respond = (id, response) => {
    assert.equal(id, 42); assert.deepEqual(response, { answers: { choice: { answers: ['one'] } } });
    attempts += 1;
    if (attempts === 1) throw new Error('simulated native write failure');
    delivered += 1;
  };
  const input = { questionId, answers: { choice: { answers: ['one'] } } };
  await assert.rejects(service.action(projectId, taskRef, 'answer', input), /simulated native write failure/);
  assert.equal(service.questions.has(questionId), true);
  await service.action(projectId, taskRef, 'answer', input);
  const resolutions = store.calls.filter((row) => row.kind === 'QUESTION_RESOLVED');
  assert.equal(resolutions.length, 2);
  assert.equal(resolutions[0].request_id, resolutions[1].request_id);
  assert.deepEqual(semanticCommand(resolutions[1]), semanticCommand(resolutions[0]), 'same resolution identity must retain all semantic fields');
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'QUESTION_RESOLVED').length, 1);
  assert.equal(attempts, 2); assert.equal(delivered, 1); assert.equal(service.questions.has(questionId), false);
});

test('continuing a saved task on a fresh service retains native progress and completion routing', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'previous-turn', input_id: 'previous-input', last_sequence: 4,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_COMPLETED' });
  const { store, codex, service } = setup(t, [executor], [
    { id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed')] },
  ]);
  let verificationStarted = 0;
  service.beginVerification = async (detail, current) => {
    assert.equal(current.turn_status, 'TURN_COMPLETED');
    assert.equal(current.input_id, 'followup-input');
    verificationStarted += 1;
  };
  await service.action(projectId, taskRef, 'continue', { inputId: 'followup-input', text: '補齊驗收證據' });
  const latest = codex.threads.get(executor.thread_id).turns.at(-1);
  codex.emit('notification', { method: 'item/completed', params: { threadId: executor.thread_id,
    turnId: latest.id, item: { type: 'agentMessage', text: '已核對保存的證據' } } });
  await Promise.allSettled([...service.operations.values()]);
  latest.status = 'completed';
  codex.emit('notification', { method: 'turn/completed', params: { threadId: executor.thread_id, turn: clone(latest) } });
  await Promise.allSettled([...service.operations.values()]);
  assert.equal(store.calls.filter((row) => row.kind === 'PROGRESS').length, 1);
  assert.equal(store.state.claims[0].turn_status, 'TURN_COMPLETED');
  assert.equal(verificationStarted, 1);
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 1);
  assert.equal(store.state.claims[0].thread_id, executor.thread_id);
});

test('reopening retries a lost durable acknowledgement by resuming the already reopened native thread', async (t) => {
  const executor = claim('EXECUTION', { archived: true, turn_id: 'saved-turn', input_id: 'saved-input', last_sequence: 5,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_COMPLETED' });
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed')] }]);
  let nativeArchived = true, unarchives = 0, rejectWrite = true;
  const resume = codex.resumeThread.bind(codex), update = store.update.bind(store);
  codex.resumeThread = async (id, options) => {
    if (nativeArchived) throw Object.assign(new Error('archived'), { code: 'CODEX_THREAD_ARCHIVED', threadId: id });
    return resume(id, options);
  };
  codex.unarchiveThread = async () => { unarchives += 1; nativeArchived = false; };
  store.update = async (command) => {
    if (command.kind === 'REOPENED' && rejectWrite) { rejectWrite = false; throw new Error('durable acknowledgement unavailable'); }
    return update(command);
  };
  await assert.rejects(service.action(projectId, taskRef, 'reopen'), /acknowledgement unavailable/);
  await service.action(projectId, taskRef, 'reopen');
  assert.equal(unarchives, 1); assert.equal(store.state.claims[0].archived, false);
  assert.equal(store.state.claims[0].turn_id, executor.turn_id);
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 0);
});
