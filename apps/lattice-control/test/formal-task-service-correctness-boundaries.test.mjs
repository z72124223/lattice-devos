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
  async readProject(project) { assert.equal(project, projectId); return { rows: [clone(this.state)] }; }
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


test('contradictory interrupted snapshot cannot permanently override an owned active turn before its real completion', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'owned-turn', input_id: 'owned-input', last_sequence: 3,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const active = nativeTurn(executor, executor.turn_id);
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [active] }]);
  const read = codex.readThread.bind(codex);
  let stale = true;
  codex.notificationSnapshot = () => [];
  codex.readThread = async (id) => {
    const result = await read(id);
    if (stale) result.turns[0].status = 'interrupted';
    return result;
  };
  assert.equal(codex.isTurnActive(executor.thread_id, executor.turn_id), true);
  await service.action(projectId, taskRef, 'reconcile');
  const first = store.state.claims[0].turn_status;
  stale = false;
  codex.threads.get(executor.thread_id).turns[0].status = 'completed';
  await service.action(projectId, taskRef, 'reconcile');
  assert.deepEqual([first, store.state.claims[0].turn_status], ['TURN_BOUND', 'TURN_COMPLETED'],
    'one contradictory read must not poison the durable terminal state and suppress later completion');
});

test('unconfirmed durable dispatch preserves original identity and exposes reconciliation status without an unproven resend', async (t) => {
  const executor = claim();
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [] }]);
  const update = store.update.bind(store);
  let lose = true;
  store.update = async (command) => {
    const result = await update(command);
    if (lose && command.kind === 'DISPATCH_STARTED') {
      lose = false;
      throw Object.assign(new Error('observation committed, acknowledgement lost'), { code: 'CONTROL_PRODUCT_OUTCOME_UNKNOWN' });
    }
    return result;
  };
  const accepted = await service.requestStart(projectId, taskRef);
  assert.equal(accepted.claims[0].claim_id, executor.claim_id);
  await Promise.allSettled([...service.operations.values()]);
  await Promise.resolve();
  assert.equal(store.state.claims[0].dispatch_started, true);
  assert.equal(store.state.claims[0].turn_id, null);
  assert.equal(codex.calls.filter((row) => row.method === 'startTurn').length, 0, 'the original process never reached native send');
  await service.close();
  const restarted = new FormalTaskService({ store, codex });
  t.after(() => restarted.close());
  await restarted.restore([projectId]);
  const sent = codex.calls.filter((row) => row.method === 'startTurn');
  assert.equal(sent.length, 0, 'an empty snapshot alone does not prove the original owner cannot still send');
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'DISPATCH_STARTED').length, 1);
  assert.equal(store.state.claims[0].thread_id, executor.thread_id);
  assert.equal(store.state.claims[0].claim_id, executor.claim_id);
  assert.equal(store.state.claims[0].input_id, executor.claim_id);
  const projected = await projectClaims(store.state.claims);
  assert.notEqual(projected.status, 'running');
  assert.equal(projected.completion_verified, false);
  assert.match(projected.progress, /核對原生回合/u);
  assert.match(projected.progress, /尚未確認執行結果/u);
});

async function projectClaims(claims) {
  const { FormalWorkStore } = await import("../src/formal-work-store.mjs");
  const page = { schema_version: 'lattice.control.product-snapshot.v1', project: { id: projectId },
    tasks: [{ task_ref: taskRef, objective: 'requested work', client_request_id: 'original-client',
      ledger: { project_id: projectId, status: 'SUBMITTED', result_digest: null, ledger_head_digest: 'b'.repeat(64) } }],
    product: { metadata: [], claims, observations: [], decisions: [] } };
  const store = new FormalWorkStore({ runtime: { call: async () => clone(page), close: async () => {} } });
  return store.detail(projectId, taskRef);
}

test('durable verification failure is projected as failed even when the bounded observation list omits it', async () => {
  const outcome = { kind: 'VERIFICATION_FAILED', summary: 'fixed verifier rejected the artifact', turn_id: 'verification-turn', sequence: 9 };
  const executor = claim('EXECUTION', { turn_id: 'execution-turn', turn_status: 'TURN_COMPLETED' });
  const verifier = claim('VERIFICATION', { turn_id: outcome.turn_id, turn_status: 'TURN_COMPLETED', verification_outcome: outcome });
  const detail = await projectClaims([executor, verifier]);
  assert.equal(detail.status, 'failed');
  assert.equal(detail.completion_verified, false);
  assert.ok(detail.progress.includes(outcome.summary));
});

test('a real completion delivered during read is accepted after the native owner clears active state', async (t) => {
  const executor = claim('EXECUTION', { turn_id: 'finishing-turn', input_id: 'finishing-input', last_sequence: 3,
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND' });
  const { store, codex, service } = setup(t, [executor], [{ id: executor.thread_id, turns: [nativeTurn(executor, executor.turn_id)] }]);
  const read = codex.readThread.bind(codex);
  codex.readThread = async (id) => {
    assert.equal(codex.isTurnActive(id, executor.turn_id), true);
    codex.threads.get(id).turns[0].status = 'completed';
    return read(id);
  };
  await service.action(projectId, taskRef, 'reconcile');
  assert.equal(store.state.claims[0].turn_status, 'TURN_COMPLETED');
  assert.equal(store.state.product.observations.filter((row) => row.kind === 'TURN_COMPLETED').length, 1);
});
