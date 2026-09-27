import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { CodexAppServer } from '../src/codex-app-server.mjs';
import { FormalTaskService } from '../src/formal-task-service.mjs';
import { elicitationMethod, elicitationDenied } from '../src/mcp-tool-elicitation.mjs';

const projectId = 'test-project', taskRef = 'a'.repeat(64);
// Sanitized native-request-1790530657192.json: only task/thread/turn identities
// are replaced. Keep the real empty schema and untrusted persistence hints.
const request = (id = 0) => ({ id, method: elicitationMethod, params: {
  threadId: 'test-thread', turnId: 'test-turn', serverName: 'lattice', mode: 'form',
  _meta: { codex_approval_kind: 'mcp_tool_call', persist: ['session', 'always'],
    tool_title: 'Read bounded LATTICE task status',
    tool_description: 'Reads durable status for one validated task reference. General tasks need only task_ref; client_request_id remains optional for legacy canary compatibility.',
    tool_params: { task_ref: taskRef }, tool_params_display: [{ name: 'task_ref', value: taskRef, display_name: 'task_ref' }] },
  message: 'Allow the lattice MCP server to run tool "lattice_task_status"?',
  requestedSchema: { type: 'object', properties: {} },
} });
const tick = () => new Promise(resolve => setImmediate(resolve));
const reorder = value => Array.isArray(value) ? value.map(reorder) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).reverse().map(key => [key, reorder(value[key])])) : value;
async function fixture(t) {
  const child = new EventEmitter();
  Object.assign(child, { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(), exitCode: null });
  child.kill = () => { child.exitCode = 0; child.emit('exit', 0, null); };
  const wire = [], send = value => child.stdout.write(JSON.stringify(value) + '\n');
  let buffered = '';
  child.stdin.on('data', chunk => {
    buffered += chunk;
    while (buffered.includes('\n')) {
      const i = buffered.indexOf('\n'), msg = JSON.parse(buffered.slice(0, i)); buffered = buffered.slice(i + 1);
      wire.push(msg);
      if (msg.method === 'initialize') send({ id: msg.id, result: { platformFamily: 'windows' } });
    }
  });
  const codex = new CodexAppServer({ codexBin: 'scripted-no-model', spawnProcess: () => child });
  await codex.connect();
  send({ method: 'turn/started', params: { threadId: 'test-thread', turn: { id: 'test-turn', status: 'inProgress' } } });
  const claim = { task_ref: taskRef, claim_id: 'test-claim', thread_id: 'test-thread', turn_id: 'test-turn',
    input_id: 'test-input', turn_status: 'TURN_BOUND', last_sequence: 1, archived: false, pending_questions: [],
    mcp_permission: { version: 1, denied: false } };
  const state = { id: taskRef, project_id: projectId, completion_verified: false, claims: [claim], product: { observations: [] } };
  const store = { state, invalidate() {}, beforeUpdate: null, afterUpdate: null,
    async detail() {
      const result = structuredClone(state);
      // Match Runtime bound_snapshot(detail=true): pending questions keep
      // payload, general observations retain only ten rows and lose payload.
      result.product.observations = result.product.observations.slice(-10).map(({ payload, ...row }) => row);
      return reorder(result);
    },
    async questionResolution(_project, _task, id) {
      return reorder(structuredClone(state.product.observations.findLast(row => row.approval_id === id && row.kind === 'QUESTION_RESOLVED') ?? null));
    },
    async update(command) {
      await this.beforeUpdate?.(command);
      assert.equal(command.expected_sequence, claim.last_sequence);
      assert.equal(command.thread_id, claim.thread_id); assert.equal(command.turn_id, claim.turn_id);
      assert.equal(command.input_id, claim.input_id); assert.equal(command.claim_id, claim.claim_id);
      assert.ok(Buffer.byteLength(JSON.stringify(command.payload)) < 16384);
      const record = { ...structuredClone(command), sequence: ++claim.last_sequence };
      state.product.observations.push(record);
      if (command.kind === 'QUESTION_REQUESTED') claim.pending_questions.push(record);
      if (command.kind === 'QUESTION_RESOLVED') claim.pending_questions = claim.pending_questions.filter(row => row.approval_id !== command.approval_id);
      if (command.kind === 'QUESTION_RESOLVED' && ['decline', 'cancel'].includes(command.payload.response.action)) claim.mcp_permission.denied = true;
      await this.afterUpdate?.(command);
      return { record: reorder(record) };
    } };
  const service = new FormalTaskService({ store, codex });
  service.owners.set('test-thread', { projectId, taskRef, claimId: claim.claim_id });
  t.after(() => service.close());
  const receive = async (message = request()) => { send(message); await tick(); await Promise.allSettled([...service.operations.values()]); };
  const answer = (id, action, extra = {}) => service.action(projectId, taskRef, 'answer', { questionId: id, action, ...extra });
  const replies = () => wire.filter(row => !row.method && (row.result || row.error));
  return { child, codex, store, service, claim, state, send, receive, answer, replies };
}

for (const action of ['accept', 'decline', 'cancel']) test(`native ${action} requires a pending durable question and an explicit one-time answer`, async t => {
  const f = await fixture(t); await f.receive();
  const id = [...f.service.questions.keys()][0];
  assert.equal(f.claim.pending_questions.length, 1); assert.equal(f.replies().length, 0);
  assert.match(f.claim.pending_questions[0].summary, /等待使用者/);
  assert.equal(JSON.stringify(f.state).includes('persist'), false);
  f.store.beforeUpdate = command => { if (command.kind === 'QUESTION_RESOLVED') assert.equal(f.replies().length, 0); };
  await f.answer(id, action);
  assert.deepEqual(f.replies(), [{ id: 0, result: { action, content: action === 'accept' ? {} : null } }]);
  assert.equal(f.claim.pending_questions.length, 0);
  assert.equal(elicitationDenied(f.state, f.claim), action !== 'accept');
  await assert.rejects(f.answer(id, action), { code: 'CONTROL_QUESTION_RECONNECT_REQUIRED' });
  await f.receive(request(1));
  assert.equal(f.replies().filter(row => row.result).length, 1, 'new native request never replays an answer');
  assert.equal(f.service.questions.size, action === 'accept' ? 1 : 0);
});

test('missing, session, metadata and foreign answers cannot authorize a request', async t => {
  const f = await fixture(t); await f.receive(); const id = [...f.service.questions.keys()][0];
  for (const input of [{}, { action: 'acceptForSession' }, { action: 'accept', _meta: { persist: 'always' } },
    { decision: 'accept' }, { action: 'accept', answers: {} }, { action: 'accept', projectId: 'wrong' }]) {
    await assert.rejects(f.service.action(projectId, taskRef, 'answer', { questionId: id, ...input }));
  }
  await assert.rejects(f.service.action('wrong', taskRef, 'answer', { questionId: id, action: 'accept' }));
  await assert.rejects(f.service.action(projectId, 'b'.repeat(64), 'answer', { questionId: id, action: 'accept' }));
  await assert.rejects(f.answer('unknown', 'accept'));
  assert.equal(f.replies().length, 0); assert.equal(f.state.product.observations.length, 1);
});

test('missing or unknown durable permission projection fails closed before and after a saved accept', async t => {
  for (const state of [undefined, { version: 2, denied: false }, { version: 1, denied: null }, { version: 1 }, { version: 1, denied: false, extra: true }]) {
    const f = await fixture(t); f.claim.mcp_permission = state; await f.receive();
    assert.equal(f.service.questions.size, 0); assert.equal(f.replies().filter(row => row.result).length, 0);
    const g = await fixture(t); await g.receive(); const id = [...g.service.questions.keys()][0];
    g.store.afterUpdate = c => { if (c.kind === 'QUESTION_RESOLVED') g.claim.mcp_permission = state; };
    await assert.rejects(g.answer(id, 'accept'), { code: 'CONTROL_ELICITATION_DENIED' });
    assert.equal(g.replies().length, 0);
  }
});

test('unsupported schemas and secret-bearing fields fail closed without retaining or echoing the input', async t => {
  const f = await fixture(t);
  const changes = [m => { m.params.mode = 'url'; }, m => { delete m.params.turnId; },
    m => { m.params.requestedSchema.properties.password = { type: 'string' }; },
    m => { m.params._meta.tool_params.task_ref = 'b'.repeat(64); }, m => { m.params.serverName = 'other'; },
    m => { m.params._meta.password = 'do-not-echo'; }, m => { m.params.message = 'do-not-echo'; },
    m => { m.params._meta.tool_description = 'do-not-echo'.repeat(2000); }, m => { m.id = 'unsupported-id'; }];
  for (const [i, change] of changes.entries()) { const msg = request(i); change(msg); await f.receive(msg); }
  assert.equal(f.replies().filter(row => row.error).length, changes.length);
  assert.equal(f.state.product.observations.length, 0); assert.equal(f.service.questions.size, 0);
  assert.equal(JSON.stringify([f.replies(), f.service.lastError]).includes('do-not-echo'), false);
});

test('identity drift during durable resolution prevents a native effect', async t => {
  for (const mutate of [f => { f.codex.connectionGeneration++; }, f => { f.codex.appServerSessionId = 'app-server-session:sha256:' + 'b'.repeat(64); },
    f => { f.claim.turn_id = 'new-turn'; }, f => { f.claim.input_id = 'new-input'; }, f => { f.claim.claim_id = 'new-claim'; },
    f => { f.claim.archived = true; }, f => { f.claim.turn_status = 'INTERRUPTED'; }, f => { f.state.completion_verified = true; },
    f => { f.codex.activeTurns.clear(); }, f => { f.service.owners.clear(); },
    f => { f.send({ method: 'serverRequest/resolved', params: { threadId: 'test-thread', requestId: 0 } }); }]) {
    const f = await fixture(t); await f.receive(); const id = [...f.service.questions.keys()][0];
    f.store.afterUpdate = c => { if (c.kind === 'QUESTION_RESOLVED') mutate(f); };
    await assert.rejects(f.answer(id, 'accept'), { code: 'CONTROL_ELICITATION_STALE' });
    assert.equal(f.replies().length, 0);
  }
});

test('durable-write uncertainty is read back only with the same explicit answer, before one native attempt', async t => {
  const f = await fixture(t); await f.receive(); const id = [...f.service.questions.keys()][0];
  f.store.beforeUpdate = c => { if (c.kind === 'QUESTION_RESOLVED') throw new Error('db unavailable'); };
  await assert.rejects(f.answer(id, 'accept')); assert.equal(f.replies().length, 0);
  f.store.beforeUpdate = null;
  f.store.afterUpdate = c => { if (c.kind === 'QUESTION_RESOLVED') throw new Error('commit acknowledgement lost'); };
  await assert.rejects(f.answer(id, 'accept')); assert.equal(f.replies().length, 0);
  f.store.afterUpdate = null;
  await assert.rejects(f.answer(id, 'decline'), { code: 'CONTROL_ELICITATION_ALREADY_RESOLVED' });
  await f.answer(id, 'accept'); assert.equal(f.replies().length, 1);
  assert.equal(f.state.product.observations.filter(row => row.kind === 'QUESTION_RESOLVED').length, 1);
});

test('uncertain native sending is never repeated, including a new service and reused native id', async t => {
  const f = await fixture(t); await f.receive(); const id = [...f.service.questions.keys()][0];
  const respond = f.codex.respond.bind(f.codex);
  f.codex.respond = (...args) => { respond(...args); throw new Error('send outcome unknown'); };
  await assert.rejects(f.answer(id, 'accept')); await assert.rejects(f.answer(id, 'accept'));
  f.codex.respond = respond;
  f.codex.off('serverRequest', f.service.onRequest); f.codex.off('notification', f.service.onNotification);
  const restarted = new FormalTaskService({ store: f.store, codex: f.codex });
  restarted.owners.set('test-thread', { projectId, taskRef, claimId: f.claim.claim_id });
  t.after(() => restarted.close());
  await assert.rejects(restarted.action(projectId, taskRef, 'answer', { questionId: id, action: 'accept' }));
  f.send(request(0)); await tick(); await Promise.allSettled([...restarted.operations.values()]);
  assert.notEqual([...restarted.questions.keys()][0], id);
  assert.equal(restarted.questions.size, 1);
  assert.equal(f.replies().filter(row => row.result).length, 1);
});

for (const action of ['decline', 'cancel']) test(`an uncertain saved accept cannot pass a later durable ${action} from a concurrent question`, async t => {
  const f = await fixture(t); await f.receive(); const first = [...f.service.questions.keys()][0];
  await f.receive(request(1)); const second = [...f.service.questions.keys()].find(id => id !== first);
  f.store.afterUpdate = c => { if (c.kind === 'QUESTION_RESOLVED' && c.approval_id === first) throw new Error('commit acknowledgement lost'); };
  await assert.rejects(f.answer(first, 'accept')); f.store.afterUpdate = null;
  await f.answer(second, action);
  // Production previews cannot reveal an older response after enough progress.
  for (let i = 0; i < 120; i++) f.state.product.observations.push({ kind: 'PROGRESS', claim_id: f.claim.claim_id });
  await assert.rejects(f.answer(first, 'accept'));
  assert.equal(f.replies().filter(row => row.result?.action === 'accept').length, 0);
});

test('connector rejects duplicate pending ids and stale callbacks without settling their replacements', async t => {
  const f = await fixture(t); await f.receive(); const identity = f.codex.serverRequestIdentity(0);
  await f.receive(); assert.equal(f.replies().at(-1).error.code, -32600);
  await f.receive(); const replacement = f.codex.serverRequestIdentity(0);
  assert.notEqual(identity, replacement);
  assert.throws(() => f.codex.respond(0, { action: 'accept' }, { requestIdentity: identity }), { code: 'CODEX_APP_SERVER_REQUEST_IDENTITY_CHANGED' });
  assert.throws(() => f.codex.rejectServerRequest(0, { requestIdentity: identity }), { code: 'CODEX_APP_SERVER_REQUEST_IDENTITY_CHANGED' });
  assert.equal(f.codex.serverRequestIdentity(0), replacement);
});

test('a queued old request, timeout, disconnect, and missing callback never grant approval', async t => {
  const f = await fixture(t);
  let release; const held = new Promise(resolve => { release = resolve; });
  const queued = f.service.serial(taskRef, () => held);
  f.send(request()); f.codex.connectionGeneration++; release(); await queued; await tick();
  assert.equal(f.service.questions.size, 0); assert.equal(f.replies().length, 0, 'old catch does not settle another generation');
  const g = await fixture(t); await g.receive(); const id = [...g.service.questions.keys()][0];
  g.codex.rejectServerRequest(0, { code: -32001, message: 'timeout' });
  await assert.rejects(g.answer(id, 'accept')); assert.equal(g.replies().filter(row => row.result).length, 0);
  const h = await fixture(t); await h.receive(); const other = [...h.service.questions.keys()][0];
  h.child.kill(); await assert.rejects(h.answer(other, 'accept'));
  const k = await fixture(t); k.service.owners.clear(); await k.receive();
  assert.equal(k.replies()[0].error.code, -32601); assert.equal(k.state.product.observations.length, 0);
});
