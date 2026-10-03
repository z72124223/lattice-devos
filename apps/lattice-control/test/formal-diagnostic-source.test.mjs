import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { FormalTaskService } from '../src/formal-task-service.mjs';
import { createLatticeServer } from '../src/server.mjs';
import { diagnosticReceiptCommand } from '../src/relative-module-receipt.mjs';
import { openCircuitSummary } from '../src/execution-recovery.mjs';

const projectId = 'source-project', taskRef = 'a'.repeat(64), clone = value => structuredClone(value);
const hash = value => createHash('sha256').update(value).digest('hex');
const command = inner => ({ command: `${JSON.stringify('C:\\Program Files\\PowerShell\\7\\pwsh.exe')} -Command ${JSON.stringify(inner)}`,
  commandActions: [{ type: 'unknown', command: inner }] });
function fixture(t, workspace = path.resolve('source-memory-only')) {
  const effects = [], calls = { detail: 0, native: 0 }, hooks = {};
  const forbidden = name => () => { effects.push(name); assert.fail(name); };
  const claim = { project_id: projectId, task_ref: taskRef, claim_id: 'claim', phase: 'EXECUTION', thread_id: 'thread',
    turn_id: 'turn', input_id: 'input', worktree_path: workspace, archived: false, dispatch_started: true,
    dispatch_sequence: 2, last_sequence: 3, turn_status: 'TURN_BOUND', pending_inputs: [], pending_questions: [], mcp_permission: { version: 1, denied: false } };
  const detail = { id: taskRef, project_id: projectId, status: 'running', completion_verified: false, ledger_head_digest: 'b'.repeat(64),
    source: { kind: 'POSTGRESQL_CONTROL_PRODUCT', authority: 'POSTGRESQL_TASK_LEDGER' },
    project: { id: projectId, active: true, project_snapshot_id: 'snapshot' },
    task: { ledger: { task_ref: taskRef, project_id: projectId, project_snapshot_id: 'snapshot', status: 'SUBMITTED' } },
    claims: [claim], product: { observations: [] } };
  const importer = path.join(workspace, 'entry.mjs');
  const failure = { id: 'failure', type: 'commandExecution', status: 'failed', exitCode: 1, ...command(`node '${importer}'`),
    aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(workspace, 'missing.mjs')}' imported from ${importer}\n`,
    cwd: workspace, durationMs: 32 };
  const marker = { type: 'userMessage', content: [{ type: 'text', text: `[LATTICE_TASK:${taskRef}:claim:input]\nprivate task text` }] };
  const turn = { id: 'turn', status: 'inProgress', items: [marker, failure] }, thread = { id: 'thread', cwd: workspace, turns: [turn] };
  const event = { sequence: 1, observedAt: new Date().toISOString(), message: { method: 'item/completed',
    params: { threadId: 'thread', turnId: 'turn', item: clone(failure) } } }, notices = [event];
  const codex = new EventEmitter();
  Object.assign(codex, { connected: true, connectionGeneration: 1, appServerSessionId: `app-server-session:sha256:${'f'.repeat(64)}`,
    pendingServerRequestCount: 0, serverRequestSequence: 0, closePromise: null, connectPromise: null,
    isTurnActive: (id, tid) => id === 'thread' && tid === 'turn' && turn.status === 'inProgress',
    notificationSnapshot: ({ threadId, turnId }) => clone(notices.filter(row =>
      row.message.params.threadId === threadId && row.message.params.turnId === turnId)), async readThread(id, options) {
      assert.equal(id, 'thread'); assert.deepEqual(options.effectIdentity, { expectedGeneration: 1, expectedSessionId: `app-server-session:sha256:${'f'.repeat(64)}` });
      calls.native++; await hooks.native?.(calls.native); return clone(thread);
    }, close: async () => {} });
  for (const name of ['connect', 'startThread', 'startTurn', 'resumeThread', 'interruptTurn', 'respond', 'request']) codex[name] = forbidden(name);
  const store = { async detail(p, task) { assert.equal(p, projectId); assert.equal(task, taskRef); calls.detail++; await hooks.detail?.(calls.detail); return clone(detail); },
    update: forbidden('store.update'), close: async () => {} };
  const service = new FormalTaskService({ store, codex, configurationLoader: forbidden('configurationLoader') });
  service.owners.set('thread', { projectId, taskRef, claimId: 'claim' });
  const selectors = { claimId: 'claim', threadId: 'thread', turnId: 'turn', failureItemId: 'failure' };
  const call = () => service.diagnosticSource(projectId, taskRef, selectors);
  const sync = () => { event.message.params.item = clone(failure); };
  t.after(async () => { await service.close(); assert.deepEqual(effects, []); });
  return { service, store, codex, claim, detail, importer, failure, marker, turn, thread, event, notices, selectors, calls, hooks, call, sync, effects };
}
const rejected = error => error.status === 409 && /^CONTROL_DIAGNOSTIC_/u.test(error.code);
async function noFileAccess(action) {
  const originals = [], attempted = [];
  for (const [object, names] of [[fs, ['readFile', 'readFileSync', 'open', 'openSync', 'writeFileSync']], [fsp, ['readFile', 'open', 'writeFile', 'mkdir']]]) {
    for (const name of names) { originals.push([object, name, object[name]]); object[name] = () => { attempted.push(name); assert.fail(name); }; }
  }
  syncBuiltinESMExports();
  try { return await action(); } finally { for (const [object, name, original] of originals) object[name] = original; syncBuiltinESMExports(); assert.deepEqual(attempted, []); }
}
test('source exports only the exact failed item, binding and fixed-age context with no filesystem or execution effects', async t => {
  const s = fixture(t), result = await noFileAccess(s.call);
  assert.equal(result.failureJson, JSON.stringify(s.failure)); assert.equal(result.failureSha256, hash(result.failureJson));
  assert.equal(result.projectRoot, s.claim.worktree_path); assert.equal(result.context.authority, 'authorized');
  assert.equal(Date.parse(result.context.validUntil), Date.parse(s.event.observedAt) + 300000);
  assert.equal(result.contextBasis, 'existing_owned_formal_execution');
  assert.equal(result.nativeSourceVerified, true); assert.equal(result.advisoryOnly, true);
  for (const key of ['adopted', 'grantsNewAuthority', 'repairAuthorized', 'producerVerified', 'diagnosticSemanticsVerified', 'lowerLevelReadCancelledOnTimeout']) assert.equal(result[key], false);
  assert.deepEqual(s.calls, { detail: 2, native: 2 }); assert.ok(!JSON.stringify(result).includes('private task text'));
});
test('unknown authority, wrong identity, pending, denied, expired and ineligible sources fail closed', async t => {
  const cases = [
    ['caller authority', s => { s.selectors.authority = 'authorized'; }], ['wrong owner', s => s.service.owners.clear()],
    ['wrong claim', s => { s.selectors.claimId = 'other'; }], ['wrong task binding', s => { s.claim.task_ref = 'b'.repeat(64); }],
    ['wrong thread', s => { s.thread.id = 'other'; }], ['wrong turn', s => { s.turn.id = 'other'; }],
    ['wrong item', s => { s.selectors.failureItemId = 'other'; }], ['wrong workspace', s => { s.thread.cwd = path.resolve('other'); }],
    ['caller authority store', s => { s.detail.source.authority = 'CALLER'; }], ['old project snapshot', s => { s.detail.task.ledger.project_snapshot_id = 'old'; }],
    ['completed', s => { s.detail.completion_verified = true; }], ['archived', s => { s.claim.archived = true; }],
    ['verification', s => { s.claim.phase = 'VERIFICATION'; }], ['pending input', s => s.claim.pending_inputs.push({})],
    ['pending question', s => s.claim.pending_questions.push({})], ['native pending', s => { s.codex.pendingServerRequestCount = 1; }],
    ['unknown permission', s => { delete s.claim.mcp_permission; }], ['denied permission', s => { s.claim.mcp_permission.denied = true; }],
    ['denied turn', s => s.service.deniedTurns.set('thread:turn', new Set(['denied']))],
    ['denied item', s => { s.failure.aggregatedOutput = 'CreateProcess rejected: blocked by policy'; s.sync(); }],
    ['circuit', s => s.detail.product.observations.push({ claim_id: 'claim', turn_id: 'turn', summary: openCircuitSummary })],
    ['missing observations', s => { delete s.detail.product.observations; }], ['missing marker', s => s.turn.items.shift()],
    ['later turn', s => s.thread.turns.push({ id: 'later', status: 'inProgress', items: [] })],
    ['missing notification', s => s.notices.pop()], ['duplicate event', s => s.notices.push(clone(s.event))],
    ['event mismatch', s => { s.event.message.params.item.durationMs++; }], ['wrong event thread', s => { s.event.message.params.threadId = 'other'; }],
    ['duplicate item', s => s.turn.items.push(clone(s.failure))], ['expired', s => { s.event.observedAt = new Date(Date.now()-300001).toISOString(); }],
    ['future event', s => { s.event.observedAt = new Date(Date.now()+60000).toISOString(); }],
    ['not failure', s => { s.failure.status = 'completed'; s.sync(); }], ['oversize raw item', s => { s.failure.extra = 'x'.repeat(1048576); s.sync(); }],
  ];
  for (const [name, mutate] of cases) await t.test(name, async c => { const s = fixture(c); mutate(s); await noFileAccess(() => assert.rejects(s.call(), rejected)); });
});
test('source and connection drift during either fresh read are rejected', async t => {
  for (const mutation of [s => { s.claim.last_sequence++; }, s => { s.detail.task.ledger.blocker = 'blocked'; },
    s => { s.codex.connectionGeneration++; }, s => { s.codex.serverRequestSequence++; },
    s => { s.failure.durationMs++; s.sync(); }, s => { s.event.observedAt = new Date(Date.now()-10000).toISOString(); }]) {
    const s = fixture(t); s.hooks.native = n => { if (n === 2) mutation(s); };
    // Claim-only changes must occur before the second detail is returned.
    s.hooks.detail = n => { if (n === 2) mutation(s); };
    await assert.rejects(s.call(), rejected);
  }
});

test('source expiry stays fixed through final validation at the five-minute boundary', async t => {
  let clock = Date.now(), accepted = 0, expired = 0;
  t.mock.method(Date, 'now', () => ++clock);
  for (let remaining = 1; remaining <= 40; remaining++) {
    const s = fixture(t);
    s.event.observedAt = new Date(clock - 300000 + remaining).toISOString();
    try {
      const result = await s.call(); accepted++;
      assert.ok(Date.parse(result.context.validUntil) > clock, 'returned context must still be current');
    } catch (error) { assert.equal(error.code, 'CONTROL_DIAGNOSTIC_SOURCE_EXPIRED'); expired++; }
  }
  assert.ok(accepted > 0 && expired > 0);
});

test('foreign notifications neither prove this source nor deny this turn', async t => {
  const s = fixture(t), foreign = clone(s.event);
  foreign.message.params.threadId = 'foreign-thread';
  foreign.message.params.item.aggregatedOutput = 'CreateProcess rejected: blocked by policy';
  s.notices.push(foreign); assert.equal((await s.call()).nativeSourceVerified, true);
  s.event.message.params.turnId = 'foreign-turn';
  await assert.rejects(s.call(), { code: 'CONTROL_DIAGNOSTIC_SOURCE_NOTIFICATION_MISMATCH' });
});
test('slow underlying read times out, releases service queue and never resumes on late resolution', async t => {
  const s = fixture(t); let release;
  s.hooks.detail = () => new Promise(resolve => { release = resolve; });
  const began = performance.now(); await assert.rejects(s.call(), { code: 'CONTROL_DIAGNOSTIC_SOURCE_DEADLINE_EXCEEDED' });
  assert.ok(performance.now()-began < 6500); assert.equal(await s.service.serial(taskRef, () => 'released'), 'released');
  release(); await new Promise(resolve => setImmediate(resolve)); assert.equal(s.calls.native, 0);
});
test('query deadline includes time queued behind another operation and prevents late source access', async t => {
  const s = fixture(t); let release;
  const blocked = s.service.serial(taskRef, () => new Promise(resolve => { release = resolve; }));
  await assert.rejects(s.call(), { code: 'CONTROL_DIAGNOSTIC_SOURCE_DEADLINE_EXCEEDED' });
  release(); await blocked; await s.service.serial(taskRef, () => {}); assert.deepEqual(s.calls, { detail: 0, native: 0 });
});
test('loopback source route uses closed selectors and propagates original source rejection', async t => {
  const s = fixture(t), app = createLatticeServer({ databasePath: ':memory:', codex: s.codex, formalWorkStore: s.store, formalTaskService: s.service,
    runtimeHealth: { current: async () => ({}), close: async () => {} }, mcpHealth: { current: async () => ({}) } });
  await new Promise(resolve => app.server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => app.server.close(resolve)));
  const base = `http://127.0.0.1:${app.server.address().port}/api/formal-work/${taskRef}/diagnosticsource?`;
  const query = new URLSearchParams({ projectId, ...s.selectors });
  const response = await fetch(base+query); assert.equal(response.status, 200); assert.equal((await response.json()).failureJson, JSON.stringify(s.failure));
  for (const extra of ['&authority=authorized', '&claimId=claim']) assert.equal((await fetch(base+query+extra)).status, 400);
  query.delete('failureItemId'); assert.equal((await fetch(base+query)).status, 200);
  const other = { ...clone(s.failure), id: 'other-failure' };
  s.turn.items.push(other); s.notices.push({ ...clone(s.event), sequence: 2, message: { method: 'item/completed', params: { threadId: 'thread', turnId: 'turn', item: other } } });
  const ambiguous = await fetch(base+query); assert.equal(ambiguous.status, 409);
  assert.deepEqual((await ambiguous.json()).failureItemIds, ['failure', 'other-failure']);
  query.set('failureItemId', 'other-failure'); assert.equal((await (await fetch(base+query)).json()).binding.failureItemId, 'other-failure');
  s.claim.mcp_permission.denied = true; assert.equal((await fetch(base+query)).status, 409);
});
test('same delivery root source -> explicit archive -> real prepare/receipt CLI -> existing receipt verifier', async t => {
  const root = await fsp.mkdtemp(path.join(tmpdir(), 'lattice-source-'));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const s = fixture(t, root); await fsp.writeFile(s.importer, "import './missing.mjs';\n");
  const app = createLatticeServer({ databasePath: ':memory:', codex: s.codex, formalWorkStore: s.store, formalTaskService: s.service,
    runtimeHealth: { current: async () => ({}), close: async () => {} }, mcpHealth: { current: async () => ({}) } });
  await new Promise(resolve => app.server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => app.server.close(resolve)));
  const base = `http://127.0.0.1:${app.server.address().port}/api/formal-work/${taskRef}`;
  const publicDetail = await (await fetch(`${base}?projectId=${projectId}`)).json(), owned = publicDetail.claims[0];
  const query = new URLSearchParams({ projectId: publicDetail.project_id, claimId: owned.claim_id, threadId: owned.thread_id, turnId: owned.turn_id });
  const sourceResponse = await fetch(`${base}/diagnosticsource?${query}`); assert.equal(sourceResponse.status, 200);
  const source = await sourceResponse.json(), failureFile = path.join(root, 'failure.json'), inputFile = path.join(root, 'prepare.json');
  await fsp.writeFile(failureFile, source.failureJson, { flag: 'wx' });
  await fsp.writeFile(inputFile, JSON.stringify({ projectRoot: source.projectRoot, importer: s.importer, failureFile,
    failureSha256: source.failureSha256, outputDirectory: path.join(root, 'receipt'), mode: 'receipt', receiptBinding: source.binding, context: source.context }));
  const cli = name => fileURLToPath(new URL(`../src/${name}.mjs`, import.meta.url));
  const run = args => execFileSync(process.execPath, ['--experimental-vm-modules', '--disable-warning=ExperimentalWarning', ...args], { encoding: 'utf8', windowsHide: true });
  const prepared = JSON.parse(run([cli('relative-module-prepare-cli'), inputFile])); assert.equal(prepared.decision, 'prepared');
  const requestPath = path.join(root, 'receipt', 'request.json'), requestSha = hash(await fsp.readFile(requestPath));
  assert.equal(prepared.command, diagnosticReceiptCommand(requestPath, requestSha));
  const output = run([cli('relative-module-diagnostic-cli'), requestPath, '--receipt', requestSha]);
  s.turn.items.push({ id: 'receipt', type: 'commandExecution', status: 'completed', exitCode: 0, ...command(prepared.command), aggregatedOutput: output });
  query.set('failureItemId', source.binding.failureItemId);
  const verifiedResponse = await fetch(`${base}/diagnostic?${query}`); assert.equal(verifiedResponse.status, 200);
  const accepted = await verifiedResponse.json();
  assert.equal(accepted.nativeClaimBindingVerified, true); assert.equal(accepted.receipt.result.advisoryOnly, true);
  assert.equal(accepted.receipt.result.trust.authorizationVerified, false);
  s.turn.items.at(-1).commandActions[0].command += '; echo altered';
  const rejectedResponse = await fetch(`${base}/diagnostic?${query}`); assert.equal(rejectedResponse.status, 409);
  assert.equal((await rejectedResponse.json()).code, 'CONTROL_DIAGNOSTIC_ITEM_REJECTED');
  await assert.rejects(s.service.relativeModuleDiagnostic(projectId, taskRef, { ...s.selectors, diagnosticItemId: 'receipt' }), { code: 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED' });
});
