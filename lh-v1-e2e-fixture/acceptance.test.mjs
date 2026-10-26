import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const fixture = dirname(fileURLToPath(import.meta.url));
const root = dirname(fixture);
const entry = join(fixture, 'entry.mjs');
const read = name => readFileSync(join(fixture, name), 'utf8');
const json = name => JSON.parse(read(name));
const sha = name => createHash('sha256').update(readFileSync(join(fixture, name))).digest('hex');
const chain = json('observed-chain.json');
const http = json('control-binding.json');
const input = json('prepare-input.json');
const request = json('receipt/request.json');
const evidence = json('receipt/evidence.json');
const preparation = JSON.parse(chain.preparation.aggregatedOutput);
const receipt = JSON.parse(chain.diagnostic.aggregatedOutput);
const binding = {
  projectId: '78cc242a-fd7c-4d77-a895-1e5ade124618',
  taskRef: '82662fd7a130df6d5e7c1e8569764758248a5f7f94ea8e2f635bd5b1087e1227',
  claimId: '559f0d7f-081f-461f-bd57-5ea59fd28dfc',
  threadId: '01a0e3f0-f086-74b1-867b-f04e17148b0b',
  turnId: '01a10208-cdd3-74a3-9705-bf8258e59bdf',
  inputId: 'LH-V1-E2E-01-attempt4-20261003',
  failureItemId: 'exec-6f003e5c-b34a-4cdf-a4b7-65860d6423d2',
};
const ids = [binding.failureItemId, 'exec-b73f2643-85a5-4714-b520-9afd2aefbf5e',
  'exec-d068fd59-0380-4797-8ed3-a21c077f8b14', 'exec-cf4e64b7-1526-4ebb-b212-89e6021c1bf4'];
const trust = { source: 'caller_supplied_snapshot', context: 'caller_asserted_current',
  sourceBytesVerified: true, importerBytesVerified: true,
  nativeProvenanceVerified: false, authorizationVerified: false };
const command = event => {
  assert.equal(event.commandActions.length, 1);
  return event.commandActions[0].command;
};

function checkChain(record) {
  assert.deepEqual(record.binding, binding);
  const events = [record.failure, record.preparation, record.diagnostic, record.success];
  assert.deepEqual(record.orderedItemIds, ids);
  assert.deepEqual(events.map(event => event.id), ids);
  assert.equal(new Set(events.map(event => event.processId)).size, 4);
  for (const [index, event] of events.entries()) {
    assert.equal(event.type, 'commandExecution');
    assert.equal(event.source, 'unifiedExecStartup');
    assert.equal(event.cwd, root);
    assert.equal(event.exitCode, index === 0 ? 1 : 0);
    assert.equal(event.status, index === 0 ? 'failed' : 'completed');
  }
  assert.deepEqual(record.failure, json('native-failure.json'));
  assert.equal(command(record.failure), `node '${entry}'`);
  assert.equal(command(record.success), command(record.failure));
  assert.equal(record.success.command, record.failure.command);
  assert.match(record.failure.aggregatedOutput, /Error \[ERR_MODULE_NOT_FOUND\]/);
  assert.ok(record.failure.aggregatedOutput.includes(`Cannot find module '${join(fixture, 'message.mjs')}' imported from ${entry}`));
  assert.equal(record.success.aggregatedOutput, 'LH_V1_E2E_OK\n');
  assert.equal(command(record.preparation), read('prepare-command.txt'));
  assert.equal(command(record.diagnostic), preparation.command);
  assert.deepEqual(record.diagnosticHttp, http);
}

function checkHttp(observation) {
  assert.equal(observation.status, 200);
  assert.equal(observation.value.nativeClaimBindingVerified, true);
  for (const key of ['producerVerified', 'inputFileBytesVerified', 'diagnosticSemanticsVerified']) {
    assert.equal(observation.value[key], false);
  }
  assert.deepEqual(observation.value.binding, binding);
  assert.deepEqual(observation.value.receipt, receipt);
  const route = new URL(observation.route, 'http://127.0.0.1'); // Parse only; no HTTP request.
  assert.equal(route.pathname, `/api/formal-work/${binding.taskRef}/diagnostic`);
  for (const key of ['projectId', 'claimId', 'threadId', 'turnId', 'failureItemId']) {
    assert.equal(route.searchParams.get(key), binding[key]);
  }
  assert.equal(route.searchParams.get('diagnosticItemId'), ids[2]);
  const at = Date.parse(observation.at);
  assert.ok(Number.isFinite(at) && at < Date.parse(receipt.validUntil));
  assert.ok(at <= Date.parse(chain.observedAt));
  // Check validity at the recorded observation, never against today's clock.
}

test('unchanged importer and single-file repair execute successfully', () => {
  assert.equal(read('entry.mjs'), "import { message } from './message.mjs';\nconsole.log(message);\n");
  assert.equal(read('message.mjs'), "export const message = 'LH_V1_E2E_OK';\n");
  assert.equal(sha('entry.mjs'), receipt.subject.importerSha256);
  const result = spawnSync(process.execPath, [entry], { cwd: root, encoding: 'utf8', timeout: 10000 });
  assert.ifError(result.error);
  assert.equal(result.signal, null);
  assert.equal(result.status, 0);
  assert.equal(result.stdout, 'LH_V1_E2E_OK\n');
  assert.equal(result.stderr, '');
});

test('recorded native failure, preparation, diagnostic and success have exact IDs and commands', () => {
  checkChain(chain);
  const helperRoot = String.raw`C:\Users\f7212\Documents\Codex\2026-07-29\verification-worktrees\life-harness-dc1b160-20260927\apps\lattice-control\src`;
  const prefix = 'node --experimental-vm-modules --disable-warning=ExperimentalWarning';
  assert.equal(read('prepare-command.txt'), `${prefix} '${helperRoot}\\relative-module-prepare-cli.mjs' '${join(fixture, 'prepare-input.json')}'`);
  assert.equal(preparation.command, `${prefix} '${helperRoot}\\relative-module-diagnostic-cli.mjs' '${join(fixture, 'receipt', 'request.json')}' --receipt ${sha('receipt/request.json')}`);
  assert.equal(preparation.decision, 'prepared');
  assert.equal(receipt.result.decision, 'selected');
  assert.equal(preparation.reason, null);
  assert.equal(receipt.result.reason, null);
});

test('saved inputs, byte hashes and receipt bind to this failure and unchanged importer', () => {
  assert.deepEqual(readdirSync(join(fixture, 'receipt')).sort(), ['evidence.json', 'request.json']);
  for (const item of [input.receiptBinding, request.receiptBinding, receipt.binding]) assert.deepEqual(item, binding);
  assert.equal(input.failureSha256, sha('native-failure.json'));
  assert.equal(input.failureFile, join(fixture, 'native-failure.json'));
  assert.equal(input.outputDirectory, join(fixture, 'receipt'));
  assert.equal(input.mode, 'receipt');
  assert.deepEqual(request.context, input.context);
  assert.equal(request.context.taskId, binding.taskRef);
  assert.equal(request.context.projectId, binding.projectId);
  assert.equal(request.context.validUntil, receipt.validUntil);
  assert.equal(receipt.requestPath, join(fixture, 'receipt', 'request.json'));
  assert.equal(receipt.requestSha256, sha('receipt/request.json'));
  assert.equal(receipt.evidenceSha256, sha('receipt/evidence.json'));
  assert.equal(request.evidenceSha256, receipt.evidenceSha256);
  assert.equal(request.evidenceFile, join(fixture, 'receipt', 'evidence.json'));
  assert.deepEqual(preparation.artifacts, { evidenceFile: request.evidenceFile,
    evidenceSha256: receipt.evidenceSha256, requestPath: receipt.requestPath, requestSha256: receipt.requestSha256 });
  assert.equal(preparation.retainedOutputDirectory, input.outputDirectory);
  for (const item of [input, request, receipt.subject, evidence.subject, receipt.result.location]) {
    assert.equal(item.projectRoot, root);
    assert.equal(item.importer, entry);
  }
  assert.equal(evidence.subject.taskId, binding.taskRef);
  assert.equal(evidence.subject.projectId, binding.projectId);
  assert.equal(evidence.subject.importerSha256, sha('entry.mjs'));
  assert.deepEqual(evidence.event, Object.fromEntries(['id', 'type', 'status', 'command', 'aggregatedOutput', 'exitCode'].map(key => [key, chain.failure[key]])));
  assert.match(receipt.failureEventSha256, /^[a-f0-9]{64}$/);
  assert.equal(receipt.result.location.specifier, './message.mjs');
  assert.equal(receipt.result.location.target, join(fixture, 'message.mjs'));
  assert.equal(receipt.result.location.nearestExistingParent, fixture);
  assert.equal(receipt.result.location.targetExistsNow, false); // Historical diagnostic, before repair.
});

test('HTTP observation was valid then; advisory and trust flags remain unchanged', () => {
  checkHttp(http);
  for (const result of [preparation, receipt.result, http.value.receipt.result]) {
    assert.equal(result.advisoryOnly, true);
    assert.equal(result.adopted, false);
    assert.deepEqual(result.trust, trust);
  }
});

test('evidence validation rejects changed commands, sequence, binding, flags and expired observation', () => {
  for (const mutate of [c => { c.success.commandActions[0].command += ' extra'; },
    c => { c.orderedItemIds.reverse(); }, c => { c.binding.claimId = 'wrong-claim'; }]) {
    const altered = structuredClone(chain);
    mutate(altered);
    assert.throws(() => checkChain(altered), assert.AssertionError);
  }
  for (const mutate of [h => { h.status = 500; }, h => { h.value.producerVerified = true; },
    h => { h.value.receipt.result.adopted = true; }, h => { h.at = receipt.validUntil; }]) {
    const altered = structuredClone(http);
    mutate(altered);
    assert.throws(() => checkHttp(altered), assert.AssertionError);
  }
});

test('readonly negative static relative import really fails without touching the repaired fixture', () => {
  const missing = './__lh_v1_readonly_missing_559f0d7f__.mjs';
  assert.equal(existsSync(join(fixture, missing)), false);
  const result = spawnSync(process.execPath, ['--input-type=module', '--eval',
    `import '${missing}'; console.log('UNREACHABLE');`], { cwd: fixture, encoding: 'utf8', timeout: 10000 });
  assert.ifError(result.error);
  assert.equal(result.signal, null);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.match(result.stderr, /Error \[ERR_MODULE_NOT_FOUND\]/);
  assert.ok(result.stderr.includes('__lh_v1_readonly_missing_559f0d7f__.mjs'));
  assert.equal(existsSync(join(fixture, missing)), false);
});
