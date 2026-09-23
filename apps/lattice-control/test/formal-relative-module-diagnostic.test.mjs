import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import fsPromises from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';

import { FormalTaskService } from '../src/formal-task-service.mjs';
import { createLatticeServer } from '../src/server.mjs';
import { diagnosticReceiptCommand, failureEventDigest } from '../src/relative-module-receipt.mjs';
import { openCircuitSummary } from '../src/execution-recovery.mjs';

const clone = value => structuredClone(value);
const projectId = 'diagnostic-memory-project', taskRef = 'a'.repeat(64);
// Deliberately nonexistent: these doubles verify binding, never local file bytes.
const workspace = path.resolve('formal-relative-diagnostic-memory-only');
const importer = path.join(workspace, 'src', 'entry.mjs');

function fixture(context) {
  const effects = [];
  const forbidden = name => (...args) => { effects.push({ name, args }); assert.fail(`forbidden effect: ${name}`); };
  const claim = { project_id: projectId, task_ref: taskRef, claim_id: 'execution-claim', phase: 'EXECUTION',
    thread_id: 'execution-thread', turn_id: 'current-turn', input_id: 'retained-input',
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND', archived: false,
    worktree_path: workspace, last_sequence: 3, pending_questions: [], pending_inputs: [] };
  const detail = {
    source: { kind: 'POSTGRESQL_CONTROL_PRODUCT', authority: 'POSTGRESQL_TASK_LEDGER' },
    id: taskRef, project_id: projectId, status: 'running', completion_verified: false,
    ledger_head_digest: 'b'.repeat(64), snapshot_revision: 'snapshot-revision-one',
    project: { id: projectId, active: true, canonical_path: path.resolve('original-memory-project'),
      project_snapshot_id: 'fixed-project-snapshot' },
    task: { ledger: { status: 'SUBMITTED', task_ref: taskRef, project_id: projectId,
      project_snapshot_id: 'fixed-project-snapshot', blocker: null, failure_code: null } },
    claims: [claim], product: { observations: [] } };
  const failure = {
    id: 'failed-import', type: 'commandExecution', status: 'failed', command: 'node src/entry.mjs',
    aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(workspace, 'src', 'missing.mjs')}' imported from ${importer}`,
    exitCode: 1 };
  const binding = { projectId, taskRef, claimId: claim.claim_id, threadId: claim.thread_id,
    turnId: claim.turn_id, inputId: claim.input_id, failureItemId: failure.id };
  const receipt = {
    schema: 'lattice.relative-module-receipt.v1', binding,
    requestPath: path.join(workspace, 'diagnostic-request.json'), requestSha256: 'c'.repeat(64),
    evidenceSha256: 'd'.repeat(64), failureEventSha256: failureEventDigest(failure),
    validUntil: new Date(Date.now() + 120_000).toISOString(),
    subject: { projectRoot: workspace, importer, importerSha256: 'e'.repeat(64) },
    result: { schema: 'lattice.relative-module-diagnostic.v1', decision: 'selected', reason: null,
      advisoryOnly: true, adopted: false,
      location: { projectRoot: workspace, importer, specifier: './missing.mjs',
        target: path.join(workspace, 'src', 'missing.mjs'), nearestExistingParent: path.join(workspace, 'src'), targetExistsNow: false },
      hints: ['Read-only fixture advice'],
      trust: { source: 'caller_supplied_snapshot', context: 'caller_asserted_current',
        sourceBytesVerified: true, importerBytesVerified: true,
        nativeProvenanceVerified: false, authorizationVerified: false } } };
  const diagnostic = { id: 'diagnostic-output', type: 'commandExecution', status: 'completed', exitCode: 0,
    command: diagnosticReceiptCommand(receipt.requestPath, receipt.requestSha256), aggregatedOutput: JSON.stringify(receipt) };
  const marker = { type: 'userMessage', content: [{ type: 'text',
    text: `[LATTICE_TASK:${taskRef}:${claim.claim_id}:${claim.input_id}]\nretained user input` }] };
  const turn = { id: claim.turn_id, status: 'inProgress', items: [marker, failure, diagnostic] };
  const thread = { id: claim.thread_id, archived: false, turns: [turn] };
  const calls = { details: 0, reads: [] }, hooks = {};
  const store = {
    async detail(project, task) {
      assert.equal(project, projectId); assert.equal(task, taskRef);
      calls.details += 1; await hooks.detail?.(calls.details); return clone(detail);
    },
    update: forbidden('store.update'), close: async () => {},
  };
  const codex = new EventEmitter();
  Object.assign(codex, {
    connected: true, ready: true, connectionGeneration: 1,
    appServerSessionId: `app-server-session:sha256:${'f'.repeat(64)}`,
    isTurnActive: (id, turnId) => id === thread.id && thread.turns.some(row => row.id === turnId && row.status === 'inProgress'),
    async readThread(id, options) {
      assert.equal(id, claim.thread_id);
      calls.reads.push(clone({ id, options })); await hooks.read?.(calls.reads.length); return clone(thread);
    },
    close: async () => {},
  });
  for (const name of ['startThread', 'startTurn', 'resumeThread', 'resumeEmptyThread', 'interruptTurn',
    'archiveThread', 'unarchiveThread', 'request', 'respond', 'listThreads']) codex[name] = forbidden(`codex.${name}`);
  const service = new FormalTaskService({ store, codex, configurationLoader: forbidden('configurationLoader') });
  service.dispatch = forbidden('service.dispatch');
  service.owners.set(claim.thread_id, { projectId, taskRef, claimId: claim.claim_id });
  context.after(() => service.close());
  const selectors = { claimId: claim.claim_id, threadId: claim.thread_id, turnId: claim.turn_id,
    failureItemId: failure.id, diagnosticItemId: diagnostic.id };
  const syncReceipt = () => { diagnostic.aggregatedOutput = JSON.stringify(receipt); };
  const call = () => service.relativeModuleDiagnostic(projectId, taskRef, selectors);
  return { claim, detail, failure, receipt, diagnostic, marker, turn, thread, store, codex, service,
    selectors, calls, hooks, effects, syncReceipt, call };
}

async function withoutFileAccess(sample, action) {
  const originals = [], accessed = [];
  for (const [object, names] of [[fsPromises, ['readFile', 'open']], [fs, ['readFile', 'readFileSync', 'open', 'openSync']]]) {
    for (const name of names) {
      originals.push([object, name, object[name]]);
      object[name] = (...args) => { accessed.push({ name, args }); assert.fail(`filesystem access forbidden: ${name}`); };
    }
  }
  syncBuiltinESMExports();
  try { return await action(); }
  finally {
    for (const [object, name, original] of originals) object[name] = original;
    syncBuiltinESMExports();
    assert.deepEqual(accessed, []); assert.deepEqual(sample.effects, []);
  }
}

const diagnosticRejection = error => error?.status === 409 && /^CONTROL_DIAGNOSTIC_/u.test(error.code ?? '');

function assertLimitedTrust(output, sample) {
  assert.equal(output.schema, 'lattice.control.relative-module-diagnostic.v1');
  assert.deepEqual(output.binding, sample.receipt.binding);
  assert.deepEqual(output.receipt, sample.receipt);
  assert.equal(output.nativeClaimBindingVerified, true);
  for (const name of ['producerVerified', 'inputFileBytesVerified', 'diagnosticSemanticsVerified']) assert.equal(output[name], false);
}

test('memory doubles bind two fresh Runtime/native snapshots without opening paths, dispatching, or writing', async context => {
  const sample = fixture(context);
  assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
  assert.equal(sample.calls.details, 2); assert.equal(sample.calls.reads.length, 2);
  for (const read of sample.calls.reads) assert.deepEqual(read.options.effectIdentity, {
    expectedGeneration: 1, expectedSessionId: sample.codex.appServerSessionId,
  });
  assert.deepEqual(sample.detail.product.observations, []);
});

const rejectedFixtures = [
  ['non-PostgreSQL authority', s => { s.detail.source.authority = 'CALLER_ASSERTED'; }],
  ['stable but stale submission snapshot', s => { s.detail.task.ledger.project_snapshot_id = 'older-project-snapshot'; }],
  ['completed task', s => { s.detail.completion_verified = true; }],
  ['archived claim', s => { s.claim.archived = true; }],
  ['verification claim', s => { s.claim.phase = 'VERIFICATION'; }],
  ['pending approval', s => { s.claim.pending_questions.push({ approval_id: 'pending' }); }],
  ['open circuit observation', s => { s.detail.product.observations.push({ claim_id: s.claim.claim_id, turn_id: s.claim.turn_id, summary: openCircuitSummary }); }],
  ['owner missing', s => { s.service.owners.clear(); }],
  ['marker missing', s => { s.turn.items.shift(); }],
  ['another current turn', s => { s.thread.turns.push({ id: 'later-turn', status: 'inProgress', items: [] }); }],
  ['diagnostic before failure', s => { s.turn.items = [s.marker, s.diagnostic, s.failure]; }],
  ['duplicate failure identity', s => { s.turn.items.push(clone(s.failure)); }],
  ['diagnostic is agent text', s => { s.diagnostic.type = 'agentMessage'; }],
  ['diagnostic unsuccessful', s => { s.diagnostic.exitCode = 1; }],
  ['diagnostic JSON has extra log text', s => { s.diagnostic.aggregatedOutput += '\nlog output'; }],
  ['diagnostic command differs', s => { s.diagnostic.command += ' --unexpected'; }],
  ['failure declined', s => { s.failure.status = 'declined'; s.failure.exitCode = null; }],
  ['failure actually completed', s => { s.failure.status = 'completed'; s.failure.exitCode = 0; }],
  ['failure policy denied', s => { s.failure.aggregatedOutput = 'CreateProcess rejected: blocked by policy'; }],
  ['failure is not commandExecution', s => { s.failure.type = 'agentMessage'; }],
  ['expired receipt', s => { s.receipt.validUntil = new Date(Date.now() - 1000).toISOString(); s.syncReceipt(); }],
  ['receipt bound to another claim', s => { s.receipt.binding.claimId = 'other-claim'; s.syncReceipt(); }],
  ['receipt subject outside worktree', s => { s.receipt.subject.projectRoot = path.resolve('unrelated-worktree'); s.syncReceipt(); }],
  ['receipt importer prefix escape', s => { s.receipt.subject.importer = path.join(`${workspace}-neighbor`, 'entry.mjs'); s.syncReceipt(); }],
  ['receipt result overclaims provenance', s => { s.receipt.result.trust.nativeProvenanceVerified = true; s.syncReceipt(); }],
  ['receipt result claims adoption', s => { s.receipt.result.adopted = true; s.syncReceipt(); }],
];

test('stale, mismatched, denied, and overclaimed memory fixtures are rejected without side effects', async context => {
  for (const [name, mutate] of rejectedFixtures) await context.test(name, async subtest => {
    const sample = fixture(subtest);
    mutate(sample);
    await withoutFileAccess(sample, () => assert.rejects(sample.call(), diagnosticRejection));
  });
});

test('connection generation and app-server session drift after every awaited read are rejected', async context => {
  for (const [source, count] of [['detail', 1], ['read', 1], ['detail', 2], ['read', 2]]) {
    for (const identity of ['generation', 'session']) await context.test(`${source}-${count}-${identity}`, async subtest => {
      const sample = fixture(subtest);
      sample.hooks[source] = async observed => {
        if (observed !== count) return;
        if (identity === 'generation') sample.codex.connectionGeneration += 1;
        else sample.codex.appServerSessionId = `app-server-session:sha256:${'9'.repeat(64)}`;
      };
      await withoutFileAccess(sample, () => assert.rejects(sample.call(), diagnosticRejection));
    });
  }
});

test('fresh claim, turn, input, and native item drift cannot reuse the first observation', async context => {
  for (const [name, hook, mutate] of [
    ['claim sequence', 'detail', s => { s.claim.last_sequence += 1; }],
    ['turn binding', 'detail', s => { s.claim.turn_id = 'replacement-turn'; }],
    ['input binding', 'detail', s => { s.claim.input_id = 'replacement-input'; }],
    ['native turn completion', 'read', s => { s.turn.status = 'completed'; }],
    ['native failure bytes', 'read', s => { s.failure.aggregatedOutput += '\nchanged'; }],
    ['native diagnostic bytes', 'read', s => { s.diagnostic.aggregatedOutput = ` ${s.diagnostic.aggregatedOutput}`; }],
  ]) await context.test(name, async subtest => {
    const sample = fixture(subtest);
    sample.hooks[hook] = async count => { if (count === 2) mutate(sample); };
    await withoutFileAccess(sample, () => assert.rejects(sample.call(), diagnosticRejection));
  });
});

async function getJson(url) {
  const response = await fetch(url, { headers: { connection: 'close' } });
  return { status: response.status, body: await response.json() };
}

test('real loopback HTTP route preserves the limited receipt and rejects changed or ambiguous selectors', async context => {
  const sample = fixture(context);
  const application = createLatticeServer({ databasePath: ':memory:', codex: sample.codex,
    formalWorkStore: sample.store, formalTaskService: sample.service,
    runtimeHealth: { current: async () => ({}), close: async () => {} },
    mcpHealth: { current: async () => ({}) } });
  await new Promise((resolve, reject) => {
    application.server.once('error', reject);
    application.server.listen(0, '127.0.0.1', resolve);
  });
  context.after(() => new Promise((resolve, reject) => application.server.close(error => error ? reject(error) : resolve())));
  const base = `http://127.0.0.1:${application.server.address().port}/api/formal-work/${taskRef}/diagnostic`;
  const query = new URLSearchParams({ projectId, ...sample.selectors });
  const success = await withoutFileAccess(sample, () => getJson(`${base}?${query}`));
  assert.equal(success.status, 200);
  assertLimitedTrust(success.body, sample);
  assert.equal(sample.calls.details, 2);
  assert.equal(sample.calls.reads.length, 2);
  const wrongClaim = new URLSearchParams(query);
  wrongClaim.set('claimId', 'another-claim');
  const rejected = await withoutFileAccess(sample, () => getJson(`${base}?${wrongClaim}`));
  assert.equal(rejected.status, 409);
  for (const suffix of ['&claimId=execution-claim', '&extra=not-supported']) {
    const malformed = await withoutFileAccess(sample, () => getJson(`${base}?${query}${suffix}`));
    assert.equal(malformed.status, 400);
  }
  sample.claim.archived = true;
  const archived = await withoutFileAccess(sample, () => getJson(`${base}?${query}`));
  assert.equal(archived.status, 409);
  assert.deepEqual(sample.detail.product.observations, []);
});
