import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';
import { parseFormalWorkArguments, runFormalWorkCommand } from '../src/formal-work-client.mjs';

const projectId = 'p1';
const taskRef = 'a'.repeat(64);
const controlOrigin = 'http://127.0.0.1:4317';
const source = { kind: 'POSTGRESQL_CONTROL_PRODUCT', authority: 'POSTGRESQL_TASK_LEDGER' };
const privateLog = 'PRIVATE_FULL_LOG_MUST_NOT_APPEAR';
const createInput = () => ({ projectId, objective: '完成指定工作', clientRequestId: 'create:stable-1' });
const continueInput = () => ({ projectId, taskRef, inputId: 'input:stable-1', text: '保留原工作，補正結果。' });

function detail(overrides = {}) {
  return {
    source, id: taskRef, project_id: projectId, project: { id: projectId },
    task: { task_ref: taskRef, ledger: { task_ref: taskRef, project_id: projectId, status: 'SUBMITTED' } },
    title: '指定工作', status: 'draft', completion_verified: false, claims: [],
    product: { observations: [{ summary: privateLog }] }, ...overrides,
  };
}

function mockHttp(value = detail(), status = 200) {
  const calls = [];
  return {
    calls,
    fetchImpl: async (url, options = {}) => {
      calls.push({ url: new URL(url), options, body: options.body ? JSON.parse(options.body) : null });
      return new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } });
    },
  };
}

async function inputFile(t, value) {
  const directory = await mkdtemp(path.join(tmpdir(), 'lattice-formal-work-client-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const file = path.join(directory, 'input.json');
  await writeFile(file, typeof value === 'string' ? value : JSON.stringify(value), 'utf8');
  return file;
}

async function invoke(command, options = {}, http = mockHttp()) {
  const result = await runFormalWorkCommand({ command, controlOrigin, requestTimeoutMs: 1000,
    ...options, fetchImpl: http.fetchImpl });
  return { result, calls: http.calls };
}

function assertOneRequest(calls, method, pathname) {
  assert.equal(calls.length, 1, 'one explicit command must make one HTTP request');
  assert.equal(calls[0].options.method ?? 'GET', method);
  assert.equal(calls[0].url.origin, controlOrigin);
  assert.equal(calls[0].url.pathname, pathname);
  assert.equal(calls[0].options.redirect, 'error', 'redirects must be rejected');
}

test('argument parser recognizes the documented command shapes', () => {
  assert.deepEqual(parseFormalWorkArguments([]), { help: true });
  assert.deepEqual(parseFormalWorkArguments(['--help']), { help: true });
  for (const command of ['state', 'projects']) {
    assert.equal(parseFormalWorkArguments([command]).command, command);
  }
  const list = parseFormalWorkArguments(['list', '--project', projectId]);
  assert.equal(list.command, 'list'); assert.equal(list.projectId, projectId);
  for (const command of ['status', 'start', 'interrupt']) {
    const parsed = parseFormalWorkArguments([command, '--project', projectId, '--task', taskRef]);
    assert.equal(parsed.command, command); assert.equal(parsed.projectId, projectId); assert.equal(parsed.taskRef, taskRef);
  }
  for (const command of ['create', 'continue']) {
    const parsed = parseFormalWorkArguments([command, '--input', 'work-input.json', '--origin', controlOrigin, '--timeout-ms', '60000']);
    assert.equal(parsed.command, command); assert.equal(parsed.inputFile, 'work-input.json');
    assert.equal(parsed.controlOrigin, controlOrigin); assert.equal(parsed.requestTimeoutMs, 60000);
  }
});

test('argument parser rejects unknown, duplicate, missing and command-incompatible flags', () => {
  const invalid = [
    ['unknown'], ['state', 'extra'], ['state', '--unknown', 'x'], ['list'],
    ['status', '--project', projectId], ['start', '--task', taskRef], ['interrupt', '--project', projectId],
    ['create'], ['continue', '--input'], ['state', '--origin'], ['state', '--timeout-ms'],
    ['list', '--project', projectId, '--project', projectId],
    ['state', '--origin', controlOrigin, '--origin', controlOrigin],
    ['state', '--timeout-ms', '1', '--timeout-ms', '2'],
    ['create', '--input', 'one.json', '--input', 'two.json'],
    ['state', '--project', projectId], ['projects', '--task', taskRef],
    ['list', '--project', projectId, '--input', 'unexpected.json'],
    ['create', '--input', 'input.json', '--task', taskRef],
    ['status', '--project', projectId, '--task', taskRef, '--input', 'input.json'],
    ['list', '--project', '--origin', controlOrigin],
  ];
  for (const argv of invalid) assert.throws(() => parseFormalWorkArguments(argv), JSON.stringify(argv));
});

test('unsafe origins and invalid timeouts are rejected before fetch', async () => {
  const origins = ['https://example.com', 'http://example.com:4317', 'http://127.0.0.1:4317.evil.example',
    'http://user:password@127.0.0.1:4317', 'http://127.0.0.1:4317/path',
    'http://127.0.0.1:4317?redirect=other', 'http://127.0.0.1:4317#fragment', 'file:///tmp/control'];
  for (const origin of origins) {
    const http = mockHttp();
    await assert.rejects(invoke('state', { controlOrigin: origin }, http), origin);
    assert.equal(http.calls.length, 0);
  }
  for (const requestTimeoutMs of [0, -1, 60001, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    const http = mockHttp();
    await assert.rejects(invoke('state', { requestTimeoutMs }, http));
    assert.equal(http.calls.length, 0);
  }
});

test('read commands do not dispatch a mutation or expose legacy logs', async () => {
  for (const command of ['state', 'projects']) {
    const http = mockHttp({ codexConnected: false, formal_work_enabled: true,
      formal_startup: { mode: 'explicit-only', restoreRequested: false },
      projects: [{ id: projectId, name: 'Project', root_path: 'C:\\fixture' }],
      workItems: [{ final_response: privateLog }], developmentRadar: { rawLog: privateLog } });
    const { result, calls } = await invoke(command, {}, http);
    assert.equal(result.operation, command);
    assertOneRequest(calls, 'GET', '/api/state');
    assert.ok(!JSON.stringify(result).includes(privateLog));
  }
  const node = { id: taskRef, title: '工作', objective: '需求', priority: 'normal', status: 'draft',
    completion_verified: false, progress: privateLog, parent_id: null, children: [], blocker: { status: 'clear', reasons: [] } };
  const http = mockHttp({ context: { status: 'ready', project_id: projectId }, formal_work_enabled: true,
    work_snapshot: { schema_version: 'lattice.control.work-snapshot.v1', project_id: projectId, source,
      revision: 'b'.repeat(64), digest: 'c'.repeat(64), tree: { nodes: [node], roots: [taskRef] }, graph: { nodes: [] } } });
  const { result, calls } = await invoke('list', { projectId }, http);
  assert.equal(result.operation, 'list'); assertOneRequest(calls, 'GET', '/api/work-view');
  assert.equal(calls[0].url.searchParams.get('projectId'), projectId);
  assert.ok(JSON.stringify(result).includes(taskRef));
  assert.ok(!JSON.stringify(result).includes(privateLog));
});

test('status preserves native identities and pending question type while omitting logs and payloads', async () => {
  const claim = { claim_id: 'claim:1', phase: 'EXECUTION', thread_id: 'thread:exact-1', turn_id: 'turn:exact-1',
    input_id: 'input:exact-1', turn_status: 'TURN_BOUND', archived: false, prompt: privateLog,
    pending_inputs: [{ input_id: 'queued:1', summary: privateLog }],
    pending_questions: [{ approval_id: 'q:1', kind: 'QUESTION_REQUESTED', summary: privateLog,
      payload: { method: 'item/tool/requestUserInput', params: { raw: privateLog } } }] };
  const { result, calls } = await invoke('status', { projectId, taskRef }, mockHttp(detail({ claims: [claim],
    progress: privateLog, last_error: { stack: privateLog }, status: 'waiting_approval' })));
  assertOneRequest(calls, 'GET', `/api/formal-work/${taskRef}`);
  assert.equal(calls[0].url.searchParams.get('projectId'), projectId);
  assert.equal(result.operation, 'status'); assert.equal(result.task.taskRef, taskRef);
  assert.equal(result.task.projectId, projectId); assert.equal(result.task.completionVerified, false);
  const actual = result.task.claims[0];
  assert.equal(actual.claimId, claim.claim_id); assert.equal(actual.threadId, claim.thread_id);
  assert.equal(actual.turnId, claim.turn_id); assert.equal(actual.inputId, claim.input_id);
  assert.equal(actual.turnStatus, claim.turn_status);
  assert.deepEqual(actual.pendingQuestions, [{ approvalId: 'q:1', kind: 'QUESTION_REQUESTED', method: 'item/tool/requestUserInput' }]);
  assert.ok(!JSON.stringify(result).includes(privateLog));
});

test('create preserves the idempotency key and never starts the returned task implicitly', async t => {
  const input = createInput(); const file = await inputFile(t, input); const http = mockHttp(detail(), 201);
  const { result, calls } = await invoke('create', { inputFile: file }, http);
  assertOneRequest(calls, 'POST', '/api/formal-work'); assert.deepEqual(calls[0].body, input);
  assert.equal(result.operation, 'create'); assert.equal(result.clientRequestId, input.clientRequestId);
  assert.equal(result.task.taskRef, taskRef); assert.equal(result.task.status, 'draft');
  assert.equal(result.task.completionVerified, false);
});

test('two explicit retries with the same create key make exactly two create requests', async t => {
  const input = createInput(); const file = await inputFile(t, input); const http = mockHttp(detail(), 201);
  await invoke('create', { inputFile: file }, http);
  await invoke('create', { inputFile: file }, http);
  assert.equal(http.calls.length, 2);
  for (const call of http.calls) {
    assert.equal(call.options.method, 'POST'); assert.equal(call.url.pathname, '/api/formal-work');
    assert.equal(call.body.clientRequestId, input.clientRequestId);
  }
});

test('start returns the acknowledgement without polling or claiming completion', async () => {
  const { result, calls } = await invoke('start', { projectId, taskRef }, mockHttp(detail({ status: 'starting' })));
  assertOneRequest(calls, 'POST', `/api/formal-work/${taskRef}/start`);
  assert.equal(calls[0].body.projectId, projectId);
  assert.equal(result.operation, 'start'); assert.equal(result.task.status, 'starting');
  assert.equal(result.task.completionVerified, false);
});

test('continue and interrupt preserve the exact work and native identities', async t => {
  const claim = { claim_id: 'claim:original', phase: 'EXECUTION', thread_id: 'thread:original', turn_id: 'turn:original',
    input_id: 'input:stable-1', turn_status: 'INTERRUPTED', archived: false, pending_questions: [] };
  const input = { ...continueInput(), phase: 'EXECUTION', text: '  保留換行\n及原文字。  ' };
  const file = await inputFile(t, input);
  for (const command of ['continue', 'interrupt']) {
    const options = command === 'continue' ? { inputFile: file } : { projectId, taskRef };
    const { result, calls } = await invoke(command, options, mockHttp(detail({ claims: [claim], status: 'failed' })));
    assertOneRequest(calls, 'POST', `/api/formal-work/${taskRef}/${command}`);
    assert.equal(calls[0].body.projectId, projectId); assert.equal(result.task.taskRef, taskRef);
    assert.equal(result.task.claims[0].threadId, claim.thread_id); assert.equal(result.task.claims[0].turnId, claim.turn_id);
    if (command === 'continue') {
      assert.equal(calls[0].body.inputId, input.inputId); assert.equal(calls[0].body.text, input.text);
      assert.equal(calls[0].body.phase, 'EXECUTION');
    }
  }
});

test('invalid selectors are rejected before any HTTP request', async () => {
  for (const badProjectId of ['', 'p'.repeat(65), 'project/name', '專案', 42]) {
    const http = mockHttp(); await assert.rejects(invoke('status', { projectId: badProjectId, taskRef }, http));
    assert.equal(http.calls.length, 0);
  }
  for (const badTaskRef of ['', 'a'.repeat(63), 'A'.repeat(64), 'g'.repeat(64), '../work', 42]) {
    const http = mockHttp(); await assert.rejects(invoke('status', { projectId, taskRef: badTaskRef }, http));
    assert.equal(http.calls.length, 0);
  }
});

test('create rejects unknown fields, invalid limits and wrong types before HTTP', async t => {
  const invalid = [
    { ...createInput(), model: 'unrequested-model' }, { ...createInput(), objective: ' ' },
    { ...createInput(), objective: '字'.repeat(513) }, { ...createInput(), objective: 10 },
    { ...createInput(), projectId: 'p'.repeat(65) }, { ...createInput(), clientRequestId: '' },
    { ...createInput(), clientRequestId: 'x'.repeat(65) }, { ...createInput(), clientRequestId: 'bad/key' },
    { ...createInput(), parentTaskRef: 'A'.repeat(64) }, { ...createInput(), priority: 4 },
    { ...createInput(), priority: 1.5 }, { ...createInput(), title: '' },
    { ...createInput(), title: '字'.repeat(81) }, { ...createInput(), successCriteria: ' ' },
    { ...createInput(), successCriteria: '字'.repeat(2731) }, [], null,
  ];
  for (const value of invalid) {
    const file = await inputFile(t, value); const http = mockHttp();
    await assert.rejects(invoke('create', { inputFile: file }, http)); assert.equal(http.calls.length, 0);
  }
});

test('valid Unicode create boundaries use code points and UTF-8 bytes correctly', async t => {
  const input = { ...createInput(), objective: '😀'.repeat(512), title: '字'.repeat(80),
    successCriteria: '字'.repeat(2730) + 'ab', priority: 3, parentTaskRef: 'b'.repeat(64) };
  const file = await inputFile(t, input); const { calls } = await invoke('create', { inputFile: file }, mockHttp(detail(), 201));
  assertOneRequest(calls, 'POST', '/api/formal-work'); assert.deepEqual(calls[0].body, input);
});

test('continue rejects invalid input identity, phase, controls and UTF-8 byte overflow', async t => {
  const invalid = [
    { ...continueInput(), model: 'unrequested-model' }, { ...continueInput(), inputId: '' },
    { ...continueInput(), inputId: 'i'.repeat(129) }, { ...continueInput(), inputId: 'bad/key' },
    { ...continueInput(), taskRef: 'A'.repeat(64) }, { ...continueInput(), phase: 'OTHER' },
    { ...continueInput(), text: ' ' }, { ...continueInput(), text: 'x\u0000y' },
    { ...continueInput(), text: '字'.repeat(5462) }, { ...continueInput(), text: 1 },
  ];
  for (const value of invalid) {
    const file = await inputFile(t, value); const http = mockHttp();
    await assert.rejects(invoke('continue', { inputFile: file }, http)); assert.equal(http.calls.length, 0);
  }
});

test('continue accepts the 16384-byte text and 128-byte input identifier boundaries', async t => {
  const input = { ...continueInput(), inputId: 'i'.repeat(128), text: '字'.repeat(5461) + 'a', phase: 'VERIFICATION' };
  const file = await inputFile(t, input); const { calls } = await invoke('continue', { inputFile: file });
  assertOneRequest(calls, 'POST', `/api/formal-work/${taskRef}/continue`);
  assert.equal(calls[0].body.inputId, input.inputId); assert.equal(calls[0].body.text, input.text);
});

test('input JSON rejects duplicates, malformed documents and files over 64 KiB', async t => {
  const invalid = [
    '{"projectId":"p1","objective":"work","clientRequestId":"first","clientRequestId":"second"}',
    '{"projectId":"p1","objective":"work","clientRequestId":"first","clientRequest\\u0049d":"second"}',
    '{"projectId":"p1",', JSON.stringify(createInput()) + ' '.repeat(65536),
  ];
  for (const content of invalid) {
    const file = await inputFile(t, content); const http = mockHttp();
    await assert.rejects(invoke('create', { inputFile: file }, http)); assert.equal(http.calls.length, 0);
  }
});

test('HTTP errors, malformed JSON and oversized responses never become successful results', async t => {
  const file = await inputFile(t, createInput());
  const cases = [
    () => new Response(JSON.stringify({ error: { code: 'REJECTED', message: privateLog } }), { status: 409 }),
    () => new Response('not JSON', { status: 200 }),
    () => new Response(JSON.stringify(detail()), { status: 200, headers: { 'content-length': String(1048577) } }),
    () => new Response(JSON.stringify({ ...detail(), ignored: 'x'.repeat(1048576) }), { status: 200 }),
    () => new Response(JSON.stringify(detail()), { status: 302, headers: { location: 'https://example.com/' } }),
  ];
  for (const response of cases) {
    let count = 0;
    await assert.rejects(runFormalWorkCommand({ command: 'create', inputFile: file, controlOrigin, requestTimeoutMs: 1000,
      fetchImpl: async () => { count += 1; return response(); } }));
    assert.equal(count, 1, 'a failed create must not retry or start work');
  }
});

test('an unexpectedly redirected successful response is rejected', async () => {
  const response = new Response(JSON.stringify(detail()), { status: 200 });
  Object.defineProperty(response, 'redirected', { value: true });
  Object.defineProperty(response, 'url', { value: 'https://example.com/other' });
  await assert.rejects(runFormalWorkCommand({ command: 'status', projectId, taskRef, controlOrigin,
    requestTimeoutMs: 1000, fetchImpl: async () => response }));
});

test('network failures and aborts are not automatically retried', async t => {
  const file = await inputFile(t, createInput());
  for (const error of [new TypeError('Network unavailable'), new DOMException('Timed out', 'AbortError')]) {
    let count = 0;
    await assert.rejects(runFormalWorkCommand({ command: 'create', inputFile: file, controlOrigin, requestTimeoutMs: 1,
      fetchImpl: async () => { count += 1; throw error; } }));
    assert.equal(count, 1);
  }
});

test('status refuses a response for another task, project or authority', async () => {
  for (const value of [detail({ id: 'b'.repeat(64) }), detail({ project_id: 'other' }),
    detail({ source: { kind: 'UNKNOWN', authority: 'SQLITE_LEGACY' } })]) {
    const http = mockHttp(value); await assert.rejects(invoke('status', { projectId, taskRef }, http));
    assert.equal(http.calls.length, 1);
  }
});

test('Life-Harness candidate progress stays advisory and omits arbitrary log fields', async () => {
  const claim = { claim_id: 'claim-a', phase: 'EXECUTION', thread_id: 'thread-a', turn_id: 'turn-a',
    turn_status: 'TURN_BOUND', pending_questions: [] };
  const candidate = { state: 'steer_intent', reason: 'RELATIVE_MODULE_FAILURE', failureIdHash: 'b'.repeat(64),
    advisoryOnly: true, adopted: false };
  const observation = { kind: 'PROGRESS', claim_id: claim.claim_id, turn_id: claim.turn_id,
    summary: `LIFE_HARNESS_CANDIDATE_V1 ${JSON.stringify(candidate)}` };
  const http = mockHttp(detail({ claims: [claim], product: { observations: [
    { ...observation, summary: privateLog }, observation,
    { ...observation, summary: `LIFE_HARNESS_CANDIDATE_V1 ${JSON.stringify({ ...candidate, adopted: true, rawLog: privateLog })}` },
    { ...observation, summary_truncated: true },
    { ...observation, claim_id: 'unrelated' },
  ] } }));
  const { result } = await invoke('status', { projectId, taskRef }, http);
  assert.equal(result.task.completionVerified, false);
  assert.equal(result.task.lifeHarness.status, 'CANDIDATE_PROGRESS_RECORDED');
  assert.equal(result.task.lifeHarness.candidates.length, 3);
  const [intent, invalid, truncated] = result.task.lifeHarness.candidates;
  for (const [key, value] of Object.entries(candidate)) assert.equal(intent[key], value);
  assert.equal(invalid.state, 'UNREADABLE_CANDIDATE'); assert.equal(truncated.state, 'UNREADABLE_CANDIDATE');
  assert.ok(!JSON.stringify(result).includes(privateLog));
});

test('state reports the served Life-Harness capability without assuming it is enabled', async () => {
  for (const enabled of [true, false, undefined]) {
    const state = { projects: [], formal_work_enabled: true,
      ...(enabled === undefined ? {} : { life_harness: { enabled, scope: 'owned-execution', mode: 'failure-candidates', rawLog: privateLog } }) };
    const { result } = await invoke('state', {}, mockHttp(state));
    assert.equal(result.lifeHarness.enabled, enabled ?? null);
    assert.equal(result.lifeHarness.scope, enabled === undefined ? null : 'owned-execution');
    assert.equal(result.lifeHarness.mode, enabled === undefined ? null : 'failure-candidates');
    assert.ok(!JSON.stringify(result).includes(privateLog));
  }
});

test('the timeout covers a real loopback response body that never finishes', async () => {
  let requests = 0;
  const server = createServer((_request, response) => {
    requests += 1;
    response.writeHead(200, { 'content-type': 'application/json' });
    response.write('{');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    await assert.rejects(runFormalWorkCommand({ command: 'state', requestTimeoutMs: 100,
      controlOrigin: `http://127.0.0.1:${server.address().port}` }), { code: 'CONTROL_REQUEST_TIMEOUT' });
    assert.equal(requests, 1);
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});

test('the CLI prints usage or one error JSON with a nonzero exit and never dispatches invalid input', async () => {
  const entrypoint = path.resolve(import.meta.dirname, '../src/formal-work-client.mjs');
  const execute = promisify(execFile);
  const help = await execute(process.execPath, [entrypoint, '--help']);
  assert.match(help.stdout, /create --input/u); assert.equal(help.stderr, '');
  await assert.rejects(execute(process.execPath, [entrypoint, 'start', '--project', projectId, '--task', 'invalid']), error => {
    assert.equal(error.code, 1); assert.equal(error.stdout, '');
    assert.equal(JSON.parse(error.stderr).status, 'ERROR');
    return true;
  });
});
