import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import fsPromises from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';


import { FormalTaskService } from "../src/formal-task-service.mjs";
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
    archived: false, pending_inputs: [], pending_questions: [], repair_attempts: 0, verification_outcome: null, ...fields };
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
    if (['QUESTION_REQUESTED', 'APPROVAL_REQUESTED'].includes(command.kind) && this.state.product.observations.some((row) => row.claim_id === command.claim_id && row.approval_id === command.approval_id)) {
      throw Object.assign(new Error('approval identity already recorded'), { code: 'CONTROL_PRODUCT_QUESTION_REJECTED' });
    }
    const current = this.state.claims.find((row) => row.claim_id === command.claim_id);
    assert.ok(current);
    assert.equal(command.expected_sequence, current.last_sequence, 'observation must use current sequence');
    const record = { ...clone(command), sequence: current.last_sequence + 1 };
    this.requests.set(command.request_id, { command: clone(command), record: clone(record) });
    this.state.product.observations.push(record);
    current.last_sequence = record.sequence;
    if (command.kind === 'THREAD_BOUND') current.thread_id = command.thread_id;
    if (command.kind === 'INPUT_QUEUED') {
      current.pending_inputs.push(record);
      current.repair_attempts = this.state.product.observations.filter((row) => row.claim_id === current.claim_id && row.kind === 'INPUT_QUEUED' && row.input_id.startsWith('repair:')).length;
    }
    if (['VERIFICATION_PASSED', 'VERIFICATION_FAILED'].includes(command.kind)) current.verification_outcome = clone(record);
    if (command.kind === 'DISPATCH_STARTED') {
      Object.assign(current, { dispatch_started: true, dispatch_sequence: record.sequence,
        turn_id: null, input_id: command.input_id, turn_status: 'DISPATCH_STARTED' });
      current.pending_inputs = current.pending_inputs.filter((row) => row.input_id !== command.input_id);
      if (current.phase === 'VERIFICATION') {
        current.execution_sequence = this.state.claims.find((row) => row.phase === 'EXECUTION').dispatch_sequence;
        current.verification_outcome = null;
      }
    }
    if (command.kind === 'TURN_BOUND') Object.assign(current, { turn_id: command.turn_id, turn_status: 'TURN_BOUND' });
    if (['TURN_COMPLETED', 'TURN_FAILED', 'INTERRUPTED', 'CLAIM_FAILED'].includes(command.kind)) current.turn_status = command.kind;
    if (['QUESTION_REQUESTED', 'APPROVAL_REQUESTED'].includes(command.kind)) current.pending_questions.push(record);
    if (['QUESTION_RESOLVED', 'APPROVAL_RESOLVED'].includes(command.kind)) current.pending_questions = current.pending_questions.filter((row) => row.approval_id !== command.approval_id);
    return { record: clone(record) };
  }
  async questionResolution(project, task, question) {
    assert.equal(project, projectId); assert.equal(task, taskRef);
    const resolution = this.state.product.observations.findLast((row) => row.approval_id === question && ['APPROVAL_RESOLVED', 'QUESTION_RESOLVED'].includes(row.kind));
    if (!resolution) return null;
    const request = this.state.product.observations.find((row) => row.approval_id === question && ['APPROVAL_REQUESTED', 'QUESTION_REQUESTED'].includes(row.kind));
    return { ...clone(resolution), method: request.payload.method };
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
  async close() {}
}
function setup(t, claims, threads) {
  const store = new MemoryStore(claims), codex = new MemoryCodex(threads);
  const service = new FormalTaskService({ store, codex, configurationLoader: async () => assert.fail('must not launch a real importer') });
  t.after(() => service.close());
  return { store, codex, service };
}

function virtualFiles(t) {
  const files = new Map([[path.join(workspace, 'artifact.mjs'), Buffer.from('artifact')], [path.join(workspace, 'artifact.test.mjs'), Buffer.from('meaningful test')]]);
  t.mock.method(fsPromises, 'realpath', async (value) => {
    const target = path.resolve(String(value)); assert.ok(target === workspace || files.has(target)); return target;
  });
  t.mock.method(fsPromises, 'readFile', async (value) => { const bytes = files.get(String(value)); assert.ok(bytes, `missing virtual file ${value}`); return bytes; });
  t.mock.method(fsPromises, 'stat', async (value) => ({ isFile: () => files.has(String(value)) }));
  t.mock.method(fsPromises, 'mkdir', async () => undefined);
  t.mock.method(fsPromises, 'writeFile', async (value, bytes, options) => {
    if (options?.flag === 'wx' && files.has(String(value))) throw Object.assign(new Error('exists'), { code: 'EEXIST' });
    files.set(String(value), Buffer.from(bytes));
  });
  syncBuiltinESMExports();
  t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports(); });
  return files;
}
function completedPair(passed = false) {
  const executor = claim('EXECUTION', { turn_id: 'execution-turn-1', input_id: 'execution-input-1',
    last_sequence: 4, dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_COMPLETED' });
  const verifier = claim('VERIFICATION', { turn_id: 'verification-turn-1', input_id: 'verification-input-1',
    last_sequence: 4, dispatch_started: true, dispatch_sequence: 2, execution_sequence: 2, turn_status: 'TURN_COMPLETED' });
  const output = { summary: 'working candidate', artifact_path: 'artifact.mjs', test_path: 'artifact.test.mjs' };
  return { executor, verifier, claims: [executor, verifier], threads: [
    { id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed', output)] },
    { id: verifier.thread_id, turns: [nativeTurn(verifier, verifier.turn_id, 'completed', { passed, summary: passed ? 'independent review passed' : 'missing required save behavior' })] },
  ] };
}
function completedNative(codex, store, phase, output) {
  const current = store.state.claims.find((row) => row.phase === phase);
  const turn = codex.threads.get(current.thread_id).turns.find((row) => row.id === current.turn_id);
  assert.equal(turn.status, 'inProgress');
  turn.status = 'completed';
  turn.items.push({ type: 'agentMessage', text: JSON.stringify(output) });
}
function attachRestart(t, store, codex) {
  const next = new FormalTaskService({ store, codex, configurationLoader: async () => assert.fail('unexpected importer') });
  t.after(() => next.close());
  return next;
}

test('resolved approval replay after restart delivers the exact saved decision without reopening the question', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'current-turn', input_id: 'current-input', last_sequence: 5,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const { store, codex, service } = setup(t, [executor], []);
  const method = 'item/commandExecution/requestApproval';
  const params = { threadId: executor.thread_id, turnId: executor.turn_id, itemId: 'same-native-item', reason: 'run a requested command' };
  const question = 'q:' + createHash('sha256').update(JSON.stringify([method, params])).digest('hex');
  store.state.product.observations.push(
    { claim_id: executor.claim_id, sequence: 4, kind: 'APPROVAL_REQUESTED', approval_id: question,
      thread_id: executor.thread_id, turn_id: executor.turn_id, input_id: executor.input_id, payload: { method, params } },
    { claim_id: executor.claim_id, sequence: 5, kind: 'APPROVAL_RESOLVED', approval_id: question,
      thread_id: executor.thread_id, turn_id: executor.turn_id, input_id: executor.input_id, approval_decision: 'accept', payload: null },
  );
  assert.equal(service.questions.size, 0, 'new process has no native request map');
  service.owners.set(executor.thread_id, { projectId, taskRef, claimId: executor.claim_id });
  codex.emit('serverRequest', { id: 901, method, params });
  await Promise.allSettled([...service.operations.values()]); await Promise.resolve();
  const responses = codex.calls.filter((row) => row.method === 'respond');
  assert.deepEqual(responses, [{ method: 'respond', id: 901, response: { decision: 'accept' } }]);
  assert.equal(codex.calls.filter((row) => row.method === 'rejectServerRequest').length, 0);
  assert.equal(store.calls.length, 0, 'a retained resolution must not create another approval request');
});

test('a saved but undispatched queued input resumes its original identity once after restart', async (t) => {
  const pending = { kind: 'INPUT_QUEUED', input_id: 'saved-followup', summary: 'implement the saved followup', sequence: 5 };
  const executor = claim('EXECUTION', { turn_id: 'finished-turn', input_id: 'first-input', last_sequence: 5,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_COMPLETED', pending_inputs: [pending] });
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id, 'completed')] }]);
  await service.action(projectId, taskRef, 'reconcile');
  const first = codex.calls.filter((row) => row.method === 'startTurn');
  assert.equal(first.length, 1); assert.equal(first[0].id, executor.thread_id);
  assert.equal(first[0].text, nativeMarker(executor, pending.input_id) + '\n' + pending.summary);
  assert.deepEqual(store.calls.map((row) => row.kind), ['DISPATCH_STARTED', 'TURN_BOUND']);
  assert.deepEqual(store.state.claims[0].pending_inputs, []);
  await service.close();
  const restarted = attachRestart(t, store, codex);
  await restarted.action(projectId, taskRef, 'reconcile');
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 1, 'bound followup must never be dispatched again');
});

test('fixed importer failure is durable after restart and does not masquerade as a completed result', { concurrency: false }, async (t) => {
  const pair = completedPair(true);
  const { store, codex, service } = setup(t, pair.claims, pair.threads);
  virtualFiles(t);
  service.configurationLoader = async () => ({ environment: { LATTICE_DELIVERY_ROOT: workspace }, executablePath: path.join(workspace, 'latticed.exe') });
  service.importResult = async () => { throw Object.assign(new Error('fixed importer unavailable'), { code: 'CONTROL_RESULT_IMPORT_FAILED' }); };
  await assert.rejects(service.action(projectId, taskRef, 'verify'), { code: 'CONTROL_RESULT_IMPORT_FAILED' });
  await service.close();
  const restarted = attachRestart(t, store, codex);
  assert.equal(restarted.lastError, undefined, 'the new process has no transient error cache');
  const detail = await store.detail(projectId, taskRef);
  const outcome = detail.claims.find((row) => row.phase === 'VERIFICATION').verification_outcome;
  assert.equal(outcome?.kind, 'VERIFICATION_FAILED', 'fixed importer failure must survive via PostgreSQL facts');
  assert.ok(outcome.summary.length > 0);
  assert.equal(detail.completion_verified, false);
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 0, 'infrastructure failure must not immediately spawn repair work');
});

test('a committed import with a lost response is recognized from fresh durable completion without retrying', { concurrency: false }, async (t) => {
  const pair = completedPair(true);
  const { store, codex, service } = setup(t, pair.claims, pair.threads);
  virtualFiles(t);
  service.configurationLoader = async () => ({ environment: { LATTICE_DELIVERY_ROOT: workspace }, executablePath: path.join(workspace, 'latticed.exe') });
  let imports = 0;
  service.importResult = async () => {
    imports += 1;
    store.state.completion_verified = true;
    store.state.result_digest = 'd'.repeat(64);
    store.state.ledger_head_digest = 'e'.repeat(64);
    throw Object.assign(new Error('import response lost after commit'), { code: 'CONTROL_RESULT_IMPORT_FAILED' });
  };
  const result = await service.action(projectId, taskRef, 'verify');
  assert.equal(result.completion_verified, true);
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'VERIFICATION_FAILED').length, 0);
  await service.close();
  const restarted = attachRestart(t, store, codex);
  restarted.importResult = async () => assert.fail('a completed result must not be imported again');
  await restarted.action(projectId, taskRef, 'verify');
  assert.equal(imports, 1); assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 0);
});

test('independent verification failure repairs on the original executor at most twice across service restarts', { concurrency: false, timeout: 5000 }, async (t) => {
  const pair = completedPair(false);
  const { store, codex, service } = setup(t, pair.claims, pair.threads);
  virtualFiles(t);
  let active = service;
  const executorStarts = () => codex.calls.filter((row) => row.method === 'startTurn' && row.id === pair.executor.thread_id);
  await active.action(projectId, taskRef, 'verify');
  assert.equal(executorStarts().length, 1, 'first verdict failure must dispatch the first bounded repair');
  assert.ok(executorStarts()[0].text.startsWith(nativeMarker(pair.executor, 'repair:verification-turn-1') + '\n'));
  assert.ok(executorStarts()[0].text.includes('missing required save behavior'));
  for (let completedRepairs = 1; completedRepairs <= 2; completedRepairs += 1) {
    await active.close();
    active = attachRestart(t, store, codex);
    completedNative(codex, store, 'EXECUTION', { summary: 'repaired implementation', artifact_path: 'artifact.mjs', test_path: 'artifact.test.mjs' });
    await active.action(projectId, taskRef, 'verify');
    const currentVerifier = store.state.claims.find((row) => row.phase === 'VERIFICATION');
    assert.equal(currentVerifier.thread_id, pair.verifier.thread_id, 'reuse the independent verification conversation');
    completedNative(codex, store, 'VERIFICATION', { passed: false, summary: 'missing required save behavior' });
    await active.action(projectId, taskRef, 'verify');
    assert.equal(executorStarts().length, Math.min(completedRepairs + 1, 2));
  }
  const retainedExecutor = store.state.claims.find((row) => row.phase === 'EXECUTION');
  assert.equal(retainedExecutor.repair_attempts, 2);
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'INPUT_QUEUED' && row.input_id.startsWith('repair:')).length, 2);
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'VERIFICATION_FAILED').length, 3);
  assert.equal(store.state.completion_verified, false);
});
