import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import fsPromises from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { FormalTaskService } from '../src/formal-task-service.mjs';
import { createLatticeServer } from '../src/server.mjs';
import { diagnosticReceiptCommand, failureEventDigest } from '../src/relative-module-receipt.mjs';
import { openCircuitSummary } from '../src/execution-recovery.mjs';

const clone = value => structuredClone(value);
const projectId = 'diagnostic-memory-project', taskRef = 'a'.repeat(64);
// Deliberately nonexistent: these doubles verify binding, never local file bytes.
const workspace = path.resolve('formal-relative-diagnostic-memory-only');
const importer = path.join(workspace, 'src', 'entry.mjs');
// Codex 851d9e9 pins shlex 1.3.0 (4a0724b0): these restricted tokens use
// DoubleQuoted display, escaping backslashes and double quotes like JSON.
const nativePrefix = '"C:\\\\Program Files\\\\PowerShell\\\\7\\\\pwsh.exe" -Command ';
const nativeCommand = inner => ({ command: nativePrefix + JSON.stringify(inner),
  commandActions: [{ type: 'unknown', command: inner }] });
const rawFailureDigest = event => createHash('sha256').update(JSON.stringify({
  id: event.id, type: event.type, status: event.status, command: event.command,
  aggregatedOutput: event.aggregatedOutput, exitCode: event.exitCode,
})).digest('hex');

function fixture(context, { verifyDiagnosticReceipt } = {}) {
  const effects = [];
  const forbidden = name => (...args) => { effects.push({ name, args }); assert.fail(`forbidden effect: ${name}`); };
  const claim = { project_id: projectId, task_ref: taskRef, claim_id: 'execution-claim', phase: 'EXECUTION',
    thread_id: 'execution-thread', turn_id: 'current-turn', input_id: 'retained-input',
    dispatch_started: true, dispatch_sequence: 2, turn_status: 'TURN_BOUND', archived: false,
    worktree_path: workspace, last_sequence: 3, pending_questions: [], pending_inputs: [],
    mcp_permission: { version: 1, denied: false } };
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
    id: 'failed-import', type: 'commandExecution', status: 'failed', ...nativeCommand(`node '${importer}'`),
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
    ...nativeCommand(diagnosticReceiptCommand(receipt.requestPath, receipt.requestSha256)), aggregatedOutput: JSON.stringify(receipt) };
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
  const service = new FormalTaskService({ store, codex, configurationLoader: forbidden('configurationLoader'), verifyDiagnosticReceipt });
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

for (const action of ['decline', 'cancel']) test(`a retained MCP ${action} in the same turn rejects diagnostic binding`, async context => {
  const sample = fixture(context);
  sample.claim.mcp_permission.denied = true;
  await withoutFileAccess(sample, () => assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_CLAIM_REJECTED' }));
  assert.equal(sample.calls.reads.length, 0);
});

for (const projection of [undefined, { version: 2, denied: false }]) test('diagnostic requires the known durable permission projection', async context => {
  const sample = fixture(context); sample.claim.mcp_permission = projection;
  await withoutFileAccess(sample, () => assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_CLAIM_REJECTED' }));
});

function assertLimitedTrust(output, sample) {
  assert.equal(output.schema, 'lattice.control.relative-module-diagnostic.v1');
  assert.deepEqual(output.binding, sample.receipt.binding);
  assert.deepEqual(output.receipt, sample.receipt);
  assert.equal(output.nativeClaimBindingVerified, true);
  for (const name of ['producerVerified', 'inputFileBytesVerified', 'diagnosticSemanticsVerified']) assert.equal(output[name], false);
  assert.equal(output.receipt.result.advisoryOnly, true);
  assert.equal(output.receipt.result.adopted, false);
  for (const name of ['nativeProvenanceVerified', 'authorizationVerified']) assert.equal(output.receipt.result.trust[name], false);
}

test('memory doubles bind two fresh Runtime/native snapshots without opening paths, dispatching, or writing', async context => {
  const sample = fixture(context);
  assert.equal(nativePrefix, `${JSON.stringify('C:\\Program Files\\PowerShell\\7\\pwsh.exe')} -Command `);
  assert.equal(sample.receipt.failureEventSha256, rawFailureDigest(sample.failure));
  assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
  assert.equal(sample.calls.details, 2); assert.equal(sample.calls.reads.length, 2);
  for (const read of sample.calls.reads) assert.deepEqual(read.options.effectIdentity, {
    expectedGeneration: 1, expectedSessionId: sample.codex.appServerSessionId,
  });
  assert.deepEqual(sample.detail.product.observations, []);
});

test('omitted diagnostic ID selects only one exact verified receipt in the current bound turn', async context => {
  const sample = fixture(context); delete sample.selectors.diagnosticItemId;
  sample.turn.items.push({ id: 'ordinary-output', type: 'commandExecution', status: 'completed', exitCode: 0,
    command: 'echo ordinary', aggregatedOutput: 'unrelated successful output' });
  assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
  assert.equal(sample.calls.details, 2); assert.equal(sample.calls.reads.length, 2);
});

test('omitted diagnostic ID rejects ambiguity, invalid receipts, prior turns and read drift without guessing', async context => {
  const cases = [
    ['two verified receipts', s => s.turn.items.push({ ...clone(s.diagnostic), id: 'other-receipt' }), 'AMBIGUOUS_RECEIPT'],
    ['duplicate verified identity', s => s.turn.items.push(clone(s.diagnostic)), 'AMBIGUOUS_RECEIPT'],
    ['same identity on unrelated item', s => s.turn.items.push({ id: s.diagnostic.id, type: 'agentMessage' }), 'ITEM_REJECTED'],
    ['no successful receipt', s => { s.diagnostic.status = 'failed'; }, 'ITEM_REJECTED'],
    ['modified producer command', s => { s.diagnostic.commandActions[0].command += '; echo injected'; }, 'ITEM_REJECTED'],
    ['other binding', s => { s.receipt.binding.failureItemId = 'other-failure'; s.syncReceipt(); }, 'ITEM_REJECTED'],
    ['expired', s => { s.receipt.validUntil = new Date(Date.now() - 1).toISOString(); s.syncReceipt(); }, 'ITEM_REJECTED'],
    ['before selected failure', s => { s.turn.items = [s.marker, s.diagnostic, s.failure]; }, 'ITEM_REJECTED'],
    ['historical receipt', s => { s.turn.items.pop(); s.thread.turns.unshift({ id: 'old-turn', status: 'completed', items: [s.diagnostic] }); }, 'ITEM_REJECTED'],
    ['receipt changed between reads', s => { s.hooks.read = n => { if (n === 2) s.diagnostic.id = 'changed-id'; }; }, 'SOURCE_CHANGED'],
    ['second matching receipt appears', s => { s.hooks.read = n => { if (n === 2) s.turn.items.push({ ...clone(s.diagnostic), id: 'new-id' }); }; }, 'AMBIGUOUS_RECEIPT'],
    ['connection changed', s => { s.hooks.read = n => { if (n === 2) s.codex.connectionGeneration++; }; }, 'CURRENTNESS_REJECTED'],
  ];
  for (const [name, mutate, code] of cases) await context.test(name, async t => {
    const sample = fixture(t); delete sample.selectors.diagnosticItemId; mutate(sample);
    await withoutFileAccess(sample, () => assert.rejects(sample.call(), { code: `CONTROL_DIAGNOSTIC_${code}` }));
  });
});

test('explicit diagnostic ID is never replaced by another valid receipt', async context => {
  const sample = fixture(context);
  sample.turn.items.push({ ...clone(sample.diagnostic), id: 'valid-other-receipt' });
  sample.diagnostic.commandActions[0].command += '; echo injected';
  await assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED' });
  sample.selectors.diagnosticItemId = 'missing-id';
  await assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_ITEM_REJECTED' });
  for (const value of ['', null, undefined]) {
    sample.selectors.diagnosticItemId = value;
    await assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_SELECTOR_REJECTED' });
  }
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

test('native display must exactly match one unknown action and the complete producer command', async context => {
  const mutations = [
    ['wrapper prefix command', (item) => { item.command = `Write-Output injected; ${item.command}`; }],
    ['wrapper suffix command', (item) => { item.command += '; Write-Output injected'; }],
    ['wrapper pipe', (item) => { item.command += ' | Write-Output'; }],
    ['extra wrapper flag', (item) => { item.command = item.command.replace(' -Command ', ' -NoProfile -Command '); }],
    ['extra wrapper argv', (item) => { item.command += ' "extra"'; }],
    ['inner prefix command', (item, inner) => Object.assign(item, nativeCommand(`Write-Output injected; ${inner}`))],
    ['inner suffix command', (item, inner) => Object.assign(item, nativeCommand(`${inner}; Write-Output injected`))],
    ['inner pipe', (item, inner) => Object.assign(item, nativeCommand(`${inner} | Write-Output`))],
    ['extra inner argv', (item, inner) => Object.assign(item, nativeCommand(`${inner} --unexpected`))],
    ['missing action', item => { delete item.commandActions; }],
    ['empty actions', item => { item.commandActions = []; }],
    ['extra action', item => { item.commandActions.push(clone(item.commandActions[0])); }],
    ['action mismatch', item => { item.commandActions[0].command += ' --unexpected'; }],
    ['action type mismatch', item => { item.commandActions[0].type = 'read'; }],
    ['extra action field', item => { item.commandActions[0].extra = true; }],
    ['bare command', (item, inner) => { item.command = inner; }],
    ['wrong shell', (item, inner) => { item.command = `${JSON.stringify('C:\\Windows\\System32\\cmd.exe')} -Command ${JSON.stringify(inner)}`; }],
    ['relative shell', (item, inner) => { item.command = `"pwsh.exe" -Command ${JSON.stringify(inner)}`; }],
    ['single quoted display', (item, inner) => { item.command = `${nativePrefix}'${inner}'`; }],
    ['unescaped Windows backslashes', item => { item.command = item.command.replaceAll('\\\\', '\\'); }],
    ['malformed ASCII apostrophe', (item, inner) => Object.assign(item, nativeCommand(inner.replace("'", "''")))],
  ];
  for (const [name, mutate] of mutations) await context.test(name, async subtest => {
    const sample = fixture(subtest);
    mutate(sample.diagnostic, sample.diagnostic.commandActions[0].command);
    await withoutFileAccess(sample, () => assert.rejects(sample.call(), {
      status: 409, code: 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED',
    }));
  });
});

test('the canonical Windows PowerShell display also preserves the limited receipt', async context => {
  const sample = fixture(context);
  const shell = 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe';
  sample.diagnostic.command = `${JSON.stringify(shell)} -Command ${JSON.stringify(sample.diagnostic.commandActions[0].command)}`;
  assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
});

test('canonical Linux native shell displays preserve the same bound advisory receipt', async context => {
  // Pinned Codex shell-command/src/bash.rs accepts [shell, -c|-lc, script].
  // shlex 1.3 emits these shell/flag tokens bare and the restricted script in
  // double quotes. These are data fixtures, not live Linux process evidence.
  for (const shell of ['/bin/bash', '/usr/bin/bash', '/bin/sh', '/usr/bin/sh', '/bin/zsh', '/usr/bin/zsh']) {
    for (const flag of ['-lc', '-c']) await context.test(`${shell} ${flag}`, async subtest => {
      const sample = fixture(subtest), inner = sample.diagnostic.commandActions[0].command;
      sample.diagnostic.command = `${shell} ${flag} ${JSON.stringify(inner)}`;
      assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
    });
  }
});

test('Linux receipt display rejects altered wrappers, argv, actions and producer source', async context => {
  const linux = inner => ({ command: `/bin/bash -lc ${JSON.stringify(inner)}`,
    commandActions: [{ type: 'unknown', command: inner }] });
  const mutations = [
    ['relative shell', item => { item.command = item.command.replace('/bin/bash', 'bash'); }],
    ['untrusted absolute shell', item => { item.command = item.command.replace('/bin/bash', '/tmp/bash'); }],
    ['shell path traversal', item => { item.command = item.command.replace('/bin/bash', '/bin/../bin/bash'); }],
    ['unsupported shell', item => { item.command = item.command.replace('/bin/bash', '/bin/fish'); }],
    ['quoted executable', item => { item.command = item.command.replace('/bin/bash', '"/bin/bash"'); }],
    ['extra wrapper flag', item => { item.command = item.command.replace(' -lc ', ' --noprofile -lc '); }],
    ['split flags', item => { item.command = item.command.replace(' -lc ', ' -l -c '); }],
    ['interactive flag', item => { item.command = item.command.replace(' -lc ', ' -ilc '); }],
    ['extra argv', item => { item.command += ' extra'; }],
    ['environment wrapper', item => { item.command = `env ${item.command}`; }],
    ['wrapper prefix', item => { item.command = `printf injected; ${item.command}`; }],
    ['wrapper suffix', item => { item.command += '; printf injected'; }],
    ['wrapper pipe', item => { item.command += ' | cat'; }],
    ['wrapper redirect', item => { item.command += ' > receipt.json'; }],
    ['inner prefix', (item, inner) => Object.assign(item, linux(`printf injected; ${inner}`))],
    ['inner suffix', (item, inner) => Object.assign(item, linux(`${inner}; printf injected`))],
    ['extra inner argv', (item, inner) => Object.assign(item, linux(`${inner} --unexpected`))],
    ['changed helper source', (item, inner) => Object.assign(item, linux(inner.replace('relative-module-diagnostic-cli.mjs', 'untrusted-cli.mjs')))],
    ['action mismatch', item => { item.commandActions[0].command += ' --unexpected'; }],
    ['extra action', item => { item.commandActions.push(clone(item.commandActions[0])); }],
  ];
  for (const [name, mutate] of mutations) await context.test(name, async subtest => {
    const sample = fixture(subtest), inner = sample.diagnostic.commandActions[0].command;
    Object.assign(sample.diagnostic, linux(inner)); mutate(sample.diagnostic, inner);
    await withoutFileAccess(sample, () => assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED' }));
  });
});

test('Linux receipts still bind the full failure command, request digest and native identity', async context => {
  for (const mutation of ['none', 'stripped failure', 'request digest', 'turn binding']) await context.test(mutation, async subtest => {
    const sample = fixture(subtest);
    sample.failure.command = `/bin/sh -c ${JSON.stringify(sample.failure.commandActions[0].command)}`;
    sample.receipt.failureEventSha256 = failureEventDigest(sample.failure);
    sample.diagnostic.command = `/bin/bash -lc ${JSON.stringify(sample.diagnostic.commandActions[0].command)}`;
    if (mutation === 'stripped failure') sample.receipt.failureEventSha256 = rawFailureDigest({
      ...sample.failure, command: sample.failure.commandActions[0].command });
    if (mutation === 'request digest') sample.receipt.requestSha256 = '0'.repeat(64);
    if (mutation === 'turn binding') sample.receipt.binding.turnId = 'another-turn';
    sample.syncReceipt();
    if (mutation === 'none') assertLimitedTrust(await withoutFileAccess(sample, sample.call), sample);
    else await withoutFileAccess(sample, () => assert.rejects(sample.call(), {
      code: mutation === 'request digest' ? 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED' : 'CONTROL_DIAGNOSTIC_BINDING_REJECTED',
    }));
  });
});

test('unsupported request path quoting fails closed', async context => {
  for (const character of ["'", '"', '\u2018', '\u2019', '\u201c', '\u201d', '$', '`', '!', '^']) {
    await context.test(`path character U+${character.codePointAt(0).toString(16)}`, async subtest => {
      const sample = fixture(subtest);
      sample.receipt.requestPath = path.join(workspace, `request${character}.json`);
      sample.syncReceipt();
      await withoutFileAccess(sample, () => assert.rejects(sample.call(), {
        status: 409, code: 'CONTROL_DIAGNOSTIC_REQUEST_REJECTED',
      }));
    });
  }
});

test('failure digest binds the original full wrapper, never its stripped inner command', async context => {
  const sample = fixture(context);
  const strippedDigest = rawFailureDigest({ ...sample.failure, command: sample.failure.commandActions[0].command });
  assert.notEqual(strippedDigest, rawFailureDigest(sample.failure));
  sample.receipt.failureEventSha256 = strippedDigest;
  sample.syncReceipt();
  await withoutFileAccess(sample, () => assert.rejects(sample.call(), {
    status: 409, code: 'CONTROL_DIAGNOSTIC_BINDING_REJECTED',
  }));
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
  const originalCommand = sample.diagnostic.command;
  sample.diagnostic.command += '; Write-Output injected';
  const malicious = await withoutFileAccess(sample, () => getJson(`${base}?${query}`));
  assert.equal(malicious.status, 409);
  sample.diagnostic.command = originalCommand;
  sample.claim.archived = true;
  const archived = await withoutFileAccess(sample, () => getJson(`${base}?${query}`));
  assert.equal(archived.status, 409);
  assert.deepEqual(sample.detail.product.observations, []);
});

// This acceptance reads an existing, explicitly supplied worktree. It never
// copies helpers or resolves a validator from a request. A supplied root must
// pass every pin check before import; an absent root is not acceptance evidence.
const fixedHelperRoot = process.env.LATTICE_DIAGNOSTIC_HELPER_ROOT;
test('trusted startup retains the pinned legacy validator through the real HTTP route',
  { skip: fixedHelperRoot ? false : 'requires LATTICE_DIAGNOSTIC_HELPER_ROOT for pinned-source acceptance' }, async context => {
    assert.ok(path.isAbsolute(fixedHelperRoot));
    const fixedHead = 'dc1b16051321788f13eb22e194d54e6d4245d652';
    const git = (...args) => execFileSync('git', ['--no-optional-locks', '-C', fixedHelperRoot, ...args],
      { windowsHide: true, timeout: 10000, maxBuffer: 1024 * 1024 });
    const text = (...args) => git(...args).toString('utf8').trim();
    const pins = [
      ['relative-module-receipt.mjs', '8d290db1d3deb6ff2f42bc4b022e485afd622cb33c9e276f7ca0e8ca2f0d1974'],
      ['execution-recovery.mjs', '8d17b4ff9d7328be7470024ef3156eeb232603d969fcd9014d951f6a09fd655a'],
      ['relative-module-diagnostic-cli.mjs', 'cda849e46c8c8a1f03191b2dfbc1be279607332bf42c16a274408723f4b102dc'],
    ];
    const verifySource = () => {
      assert.equal(fs.realpathSync(text('rev-parse', '--show-toplevel')), fs.realpathSync(fixedHelperRoot));
      assert.equal(text('rev-parse', 'HEAD'), fixedHead);
      assert.equal(text('status', '--porcelain=v1', '--untracked-files=all'), '');
      for (const [name, sha256] of pins) {
        const relative = `apps/lattice-control/src/${name}`;
        const bytes = fs.readFileSync(path.join(fixedHelperRoot, relative));
        assert.deepEqual(bytes, git('show', `${fixedHead}:${relative}`));
        assert.equal(createHash('sha256').update(bytes).digest('hex'), sha256);
      }
    };
    verifySource();
    context.after(verifySource);
    const legacy = await import(pathToFileURL(path.join(fixedHelperRoot, 'apps/lattice-control/src/relative-module-receipt.mjs')).href);
    const useLegacyCommand = sample => Object.assign(sample.diagnostic,
      nativeCommand(legacy.diagnosticReceiptCommand(sample.receipt.requestPath, sample.receipt.requestSha256)));

    await context.test('startup accepts a function dependency, never a module path or receipt object', async subtest => {
      const sample = fixture(subtest);
      const listeners = sample.codex.listenerCount('notification');
      for (const value of [null, {}, pathToFileURL(path.join(fixedHelperRoot, 'apps/lattice-control/src/relative-module-receipt.mjs')).href]) {
        assert.throws(() => new FormalTaskService({ store: sample.store, codex: sample.codex, verifyDiagnosticReceipt: value }), TypeError);
        assert.equal(sample.codex.listenerCount('notification'), listeners);
      }
      assert.deepEqual(sample.effects, []);
    });

    await context.test('the default real validator remains source-specific', async subtest => {
      const sample = fixture(subtest);
      assertLimitedTrust(await sample.call(), sample);
      useLegacyCommand(sample);
      await assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED' });
      assert.deepEqual(sample.effects, []);
    });

    for (const [name, mutate, code] of [
      ['exact old helper command', () => {}, null],
      ['current helper command', s => Object.assign(s.diagnostic, nativeCommand(diagnosticReceiptCommand(s.receipt.requestPath, s.receipt.requestSha256))), 'COMMAND_REJECTED'],
      ['another helper directory', s => Object.assign(s.diagnostic, nativeCommand(s.diagnostic.commandActions[0].command.replace(path.resolve(fixedHelperRoot), path.resolve('untrusted-helper')))), 'COMMAND_REJECTED'],
      ['appended command', s => Object.assign(s.diagnostic, nativeCommand(`${s.diagnostic.commandActions[0].command}; Write-Output injected`)), 'COMMAND_REJECTED'],
      ['wrong binding', s => { s.receipt.binding.claimId = 'other-claim'; s.syncReceipt(); }, 'BINDING_REJECTED'],
      ['expired receipt', s => { s.receipt.validUntil = new Date(Date.now() - 1000).toISOString(); s.syncReceipt(); }, 'EXPIRED'],
      ['expiry beyond five minutes', s => { s.receipt.validUntil = new Date(Date.now() + 360000).toISOString(); s.syncReceipt(); }, 'EXPIRED'],
    ]) await context.test(name, async subtest => {
      const sample = fixture(subtest, { verifyDiagnosticReceipt: legacy.verifyDiagnosticReceipt });
      useLegacyCommand(sample);
      assert.notEqual(sample.diagnostic.commandActions[0].command, diagnosticReceiptCommand(sample.receipt.requestPath, sample.receipt.requestSha256));
      mutate(sample);
      if (code) await assert.rejects(sample.call(), { code: `CONTROL_DIAGNOSTIC_${code}` });
      else assertLimitedTrust(await sample.call(), sample);
      assert.deepEqual(sample.effects, []);
    });

    for (const permission of [{ version: 1, denied: true }, undefined, { version: 2, denied: false }]) {
      await context.test(`durable permission guard precedes native receipt validation: ${JSON.stringify(permission)}`, async subtest => {
        const sample = fixture(subtest, { verifyDiagnosticReceipt: legacy.verifyDiagnosticReceipt });
        sample.claim.mcp_permission = permission;
        sample.diagnostic.aggregatedOutput = 'invalid receipt';
        await assert.rejects(sample.call(), { code: 'CONTROL_DIAGNOSTIC_CLAIM_REJECTED' });
        assert.equal(sample.calls.reads.length, 0);
        assert.deepEqual(sample.effects, []);
      });
    }

    await context.test('HTTP and receipt fields cannot select or replace the startup dependency', async subtest => {
      const sample = fixture(subtest, { verifyDiagnosticReceipt: legacy.verifyDiagnosticReceipt });
      useLegacyCommand(sample);
      const application = createLatticeServer({ databasePath: ':memory:', codex: sample.codex,
        formalWorkStore: sample.store, formalTaskService: sample.service,
        runtimeHealth: { current: async () => ({}), close: async () => {} },
        mcpHealth: { current: async () => ({}) } });
      await new Promise((resolve, reject) => {
        application.server.once('error', reject);
        application.server.listen(0, '127.0.0.1', resolve);
      });
      subtest.after(() => new Promise((resolve, reject) => application.server.close(error => error ? reject(error) : resolve())));
      const query = new URLSearchParams({ projectId, ...sample.selectors });
      const base = `http://127.0.0.1:${application.server.address().port}/api/formal-work/${taskRef}/diagnostic`;
      const success = await getJson(`${base}?${query}`);
      assert.equal(success.status, 200);
      assertLimitedTrust(success.body, sample);
      for (const key of ['verifyDiagnosticReceipt', 'helperSourceRoot', 'validatorPath']) {
        const injected = new URLSearchParams(query);
        injected.set(key, pathToFileURL(path.resolve('untrusted-helper.mjs')).href);
        const rejected = await getJson(`${base}?${injected}`);
        assert.equal(rejected.status, 400);
        assert.equal(rejected.body.code, 'CONTROL_DIAGNOSTIC_SELECTOR_REJECTED');
        sample.receipt[key] = injected.get(key);
        sample.syncReceipt();
        const receiptRejected = await getJson(`${base}?${query}`);
        assert.equal(receiptRejected.status, 409);
        assert.equal(receiptRejected.body.code, 'CONTROL_DIAGNOSTIC_BINDING_REJECTED');
        delete sample.receipt[key]; sample.syncReceipt();
      }
      Object.assign(sample.diagnostic, nativeCommand(diagnosticReceiptCommand(sample.receipt.requestPath, sample.receipt.requestSha256)));
      const wrongSource = await getJson(`${base}?${query}`);
      assert.equal(wrongSource.status, 409);
      assert.equal(wrongSource.body.code, 'CONTROL_DIAGNOSTIC_COMMAND_REJECTED');
      assert.deepEqual(sample.effects, []);
    });
    context.diagnostic(`Pinned helper verified before and after: ${fixedHead}; ${pins.map(([name, hash]) => `${name}=${hash}`).join('; ')}`);
  });
