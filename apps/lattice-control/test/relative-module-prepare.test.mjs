import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, writeFileSync } from 'node:fs';
import { lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { prepareRelativeModule } from '../src/relative-module-diagnostic.mjs';

const execute = promisify(execFile), sha = bytes => createHash('sha256').update(bytes).digest('hex');
const prepCli = fileURLToPath(new URL('../src/relative-module-prepare-cli.mjs', import.meta.url));
const diagnosticCli = fileURLToPath(new URL('../src/relative-module-diagnostic-cli.mjs', import.meta.url));
const flags = ['--experimental-vm-modules', '--disable-warning=ExperimentalWarning'];
async function fixture(t, specifier = './absent-helper.mjs') {
  const directory = await realpath(await mkdtemp(path.join(tmpdir(), 'lattice-prepare-'))), links = [];
  t.after(async () => {
    assert.equal(await realpath(directory), directory);
    for (const link of links.reverse()) {
      assert.equal(path.relative(directory, link).startsWith('..'), false);
      assert.equal((await lstat(link)).isSymbolicLink(), true);
      await unlink(link);
    }
    assert.equal(await realpath(directory), directory);
    await rm(directory, { recursive: true });
  });
  const root = path.join(directory, 'project'); await mkdir(root);
  const importer = path.join(root, 'loader.mjs'), failureFile = path.join(directory, 'event.json');
  await writeFile(importer, `import ${JSON.stringify(specifier)};\nthrow new Error('never evaluate source');\n`);
  const target = path.resolve(root, specifier), sentinel = 'DO_NOT_PRINT_RAW_FAILURE_SECRET';
  const event = { id: 'controlled-failure', type: 'commandExecution', status: 'failed', command: `node '${importer}'`,
    aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${target}' imported from ${importer}\n${sentinel}`, exitCode: 1 };
  const input = { projectRoot: root, importer, failureFile, failureSha256: '', outputDirectory: path.join(directory, 'new-output'),
    context: { projectId: 'project', taskId: 'a'.repeat(64), authority: 'authorized', projectScope: 'matched',
      lifecycle: 'active', freshness: 'current', circuitOpen: false, validUntil: new Date(Date.now() + 120000).toISOString() } };
  const binding = { projectId: input.context.projectId, taskRef: input.context.taskId, claimId: 'claim',
    threadId: 'thread', turnId: 'turn', inputId: 'input', failureItemId: event.id };
  const saveEvent = async () => { const bytes = JSON.stringify(event) + '\n'; await writeFile(failureFile, bytes); input.failureSha256 = sha(bytes); };
  await saveEvent();
  return { directory, root, input, event, binding, target, sentinel, saveEvent, links };
}
function rejected(result) {
  assert.notEqual(result.decision, 'prepared'); assert.equal(result.command, null); assert.equal(result.artifacts, null);
  assert.equal(result.advisoryOnly, true); assert.equal(result.adopted, false);
  assert.equal(result.trust.nativeProvenanceVerified, false); assert.equal(result.trust.authorizationVerified, false);
}

test('preparation CLI feeds original diagnostic CLI using a real controlled Node failure; never repairs or runs supplied command', async t => {
  const s = await fixture(t);
  await assert.rejects(execute(process.execPath, [s.input.importer], { windowsHide: true }), error => {
    s.event.aggregatedOutput = error.stderr; s.event.exitCode = error.code; return error.code === 1;
  });
  const marker = path.join(s.directory, 'must-not-execute');
  s.event.command = `node -e "require('fs').writeFileSync('${marker.replaceAll('\\', '/')}', 'bad')"`;
  await s.saveEvent();
  const config = path.join(s.directory, 'prepare.json'); await writeFile(config, JSON.stringify(s.input));
  const prepared = JSON.parse((await execute(process.execPath, [...flags, prepCli, config], { windowsHide: true })).stdout);
  assert.equal(prepared.decision, 'prepared'); assert.equal(existsSync(marker), false);
  assert.equal(prepared.command, `node --experimental-vm-modules --disable-warning=ExperimentalWarning '${diagnosticCli}' '${prepared.artifacts.requestPath}'`);
  const requestBytes = await readFile(prepared.artifacts.requestPath), request = JSON.parse(requestBytes);
  assert.equal(sha(requestBytes), prepared.artifacts.requestSha256);
  assert.deepEqual(request.context, s.input.context); assert.equal(request.receiptBinding, undefined);
  assert.equal(sha(await readFile(request.evidenceFile)), request.evidenceSha256);
  const result = JSON.parse((await execute(process.execPath, [...flags, diagnosticCli, prepared.artifacts.requestPath], { windowsHide: true })).stdout);
  assert.equal(result.decision, 'selected'); assert.equal(result.location.target, s.target);
  assert.equal(result.trust.nativeProvenanceVerified, false); assert.equal(result.trust.authorizationVerified, false);
  assert.equal(existsSync(s.target), false);
});

test('explicit receipt mode preserves full binding and produces the existing exact standalone command', async t => {
  const s = await fixture(t); s.input.mode = 'receipt'; s.input.receiptBinding = s.binding;
  const before = structuredClone(s.input), result = await prepareRelativeModule(s.input);
  assert.deepEqual(s.input, before); assert.equal(result.decision, 'prepared');
  assert.equal(JSON.stringify(result).includes(s.sentinel), false);
  const run = await execute(process.execPath, [...flags, diagnosticCli, result.artifacts.requestPath, '--receipt', result.artifacts.requestSha256], { windowsHide: true });
  const receipt = JSON.parse(run.stdout);
  assert.deepEqual(receipt.binding, s.binding); assert.equal(receipt.result.decision, 'selected');
  assert.equal(receipt.validUntil, s.input.context.validUntil);
  assert.equal(result.command, `node --experimental-vm-modules --disable-warning=ExperimentalWarning '${diagnosticCli}' '${result.artifacts.requestPath}' --receipt ${result.artifacts.requestSha256}`);
});

test('context exclusions precede all event/importer reads and output writes', async t => {
  for (const [field, value] of [['authority','denied'], ['projectScope','unknown'], ['lifecycle','retired'],
    ['validUntil','2000-01-01T00:00:00Z'], ['circuitOpen',true], ['freshness','unknown']]) await t.test(field, async child => {
    const s = await fixture(child); s.input.context[field] = value;
    await unlink(s.input.failureFile); await unlink(s.input.importer);
    const result = await prepareRelativeModule(s.input); rejected(result);
    assert.equal(result.decision, 'excluded'); assert.equal(existsSync(s.input.outputDirectory), false);
  });
});

test('missing evidence, wrong hashes and wrong events never produce output', async t => {
  for (const kind of ['missing-hash','tampered-bytes','missing-file','success','wrong-type','exit-zero','denied','wrong-importer','unsupported','oversize','oversize-importer']) await t.test(kind, async child => {
    const s = await fixture(child);
    if (kind === 'success') s.event.status = 'completed';
    if (kind === 'wrong-type') s.event.type = 'agentMessage';
    if (kind === 'exit-zero') s.event.exitCode = 0;
    if (kind === 'denied') s.event.aggregatedOutput = 'Command rejected by user';
    if (kind === 'wrong-importer') s.event.aggregatedOutput = s.event.aggregatedOutput.replace(s.input.importer, path.join(s.directory, 'other.mjs'));
    if (kind === 'unsupported') s.event.aggregatedOutput = 'unrelated failure';
    if (kind === 'oversize') s.event.aggregatedOutput = 'x'.repeat(1024 * 1024 + 1);
    await s.saveEvent();
    if (kind === 'missing-hash') delete s.input.failureSha256;
    if (kind === 'tampered-bytes') await writeFile(s.input.failureFile, (await readFile(s.input.failureFile)) + ' ');
    if (kind === 'missing-file') await unlink(s.input.failureFile);
    if (kind === 'oversize-importer') await writeFile(s.input.importer, 'x'.repeat(1024 * 1024 + 1));
    const result = await prepareRelativeModule(s.input); rejected(result);
    assert.equal(existsSync(s.input.outputDirectory), false); assert.equal(JSON.stringify(result).includes(s.sentinel), false);
  });
});

test('receipt never invents, ignores, or mismatches binding or extends its time window', async t => {
  for (const kind of ['missing','incomplete','project','task','event','plain','too-long']) await t.test(kind, async child => {
    const s = await fixture(child); s.input.mode = 'receipt'; s.input.receiptBinding = { ...s.binding };
    if (kind === 'missing') delete s.input.receiptBinding;
    if (kind === 'incomplete') delete s.input.receiptBinding.turnId;
    if (kind === 'project') s.input.receiptBinding.projectId = 'other';
    if (kind === 'task') s.input.receiptBinding.taskRef = 'b'.repeat(64);
    if (kind === 'event') s.input.receiptBinding.failureItemId = 'other';
    if (kind === 'plain') s.input.mode = 'diagnostic';
    if (kind === 'too-long') s.input.context.validUntil = new Date(Date.now() + 600000).toISOString();
    const result = await prepareRelativeModule(s.input); rejected(result); assert.equal(existsSync(s.input.outputDirectory), false);
  });
});

test('rejects traversal, import targets outside the project, unsafe command paths and output/target collisions', async t => {
  for (const kind of ['importer','relative-target','shell-path','target-dir','target-artifact']) await t.test(kind, async child => {
    const s = await fixture(child, kind === 'relative-target' ? '../outside.mjs' : kind === 'target-artifact' ? './bundle/request.json' : './absent-helper.mjs');
    if (kind === 'importer') s.input.importer = path.join(s.directory, 'outside.mjs');
    if (kind === 'shell-path') s.input.outputDirectory = path.join(s.directory, 'unsafe$path');
    if (kind === 'target-dir') s.input.outputDirectory = s.target;
    if (kind === 'target-artifact') s.input.outputDirectory = path.dirname(s.target);
    const result = await prepareRelativeModule(s.input); rejected(result); assert.equal(existsSync(s.input.outputDirectory), false);
  });
});

test('existing output is never reused or overwritten', async t => {
  const s = await fixture(t); await mkdir(s.input.outputDirectory);
  const retained = path.join(s.input.outputDirectory, 'request.json'); await writeFile(retained, 'existing user bytes');
  const result = await prepareRelativeModule(s.input); rejected(result); assert.equal(result.reason, 'OUTPUT_EXISTS');
  assert.equal(await readFile(retained, 'utf8'), 'existing user bytes');
});

test('source drift and mid-write expiry retain partial evidence and withhold the command', async t => {
  for (const kind of ['event-drift','importer-drift','expiry']) await t.test(kind, async child => {
    const s = await fixture(child); let changed = false;
    const result = await prepareRelativeModule(s.input, { now: () => {
      if (existsSync(path.join(s.input.outputDirectory, 'evidence.json'))) {
        if (kind === 'expiry') return Date.parse(s.input.context.validUntil) + 1;
        if (!changed) { writeFileSync(kind === 'event-drift' ? s.input.failureFile : s.input.importer, 'changed bytes'); changed = true; }
      }
      return Date.now();
    } });
    rejected(result); assert.equal(result.retainedOutputDirectory, s.input.outputDirectory);
    assert.equal(result.reason, kind === 'expiry' ? 'CONTEXT_EXPIRED_OR_CHANGED' : 'SOURCE_CHANGED');
    assert.equal(existsSync(path.join(s.input.outputDirectory, 'evidence.json')), true);
    if (kind === 'expiry') assert.equal(existsSync(path.join(s.input.outputDirectory, 'request.json')), false);
  });
});

test('junction or symlink ancestors do not grant event, importer, or output access', async t => {
  const s = await fixture(t), link = path.join(s.directory, 'redirect');
  try { await symlink(s.root, link, process.platform === 'win32' ? 'junction' : 'dir'); }
  catch (error) { if (['EPERM','EACCES','ENOTSUP'].includes(error.code)) return t.skip(error.code); throw error; }
  s.links.push(link);
  for (const field of ['projectRoot','failureFile','outputDirectory']) {
    await s.saveEvent();
    const input = structuredClone(s.input);
    if (field === 'projectRoot') {
      input.projectRoot = link; input.importer = path.join(link, 'loader.mjs');
      const bytes = JSON.stringify({ ...s.event, aggregatedOutput: s.event.aggregatedOutput.replaceAll(s.root, link) });
      await writeFile(input.failureFile, bytes); input.failureSha256 = sha(bytes);
    }
    if (field === 'failureFile') {
      await writeFile(path.join(s.root, 'event.json'), await readFile(s.input.failureFile));
      input.failureFile = path.join(link, 'event.json');
    }
    if (field === 'outputDirectory') input.outputDirectory = path.join(link, 'output');
    const result = await prepareRelativeModule(input); rejected(result);
    assert.match(result.reason, /REDIRECTED/u);
    assert.equal(existsSync(path.join(s.root, 'output')), false);
  }
});

test('CLI errors never echo malformed configuration or raw event contents', async t => {
  const s = await fixture(t), config = path.join(s.directory, 'malformed.json');
  await writeFile(config, `{"secret":"${s.sentinel}`);
  await assert.rejects(execute(process.execPath, [...flags, prepCli, config], { windowsHide: true }), error => {
    assert.equal(error.stdout, ''); assert.equal(error.stderr.includes(s.sentinel), false);
    assert.deepEqual(JSON.parse(error.stderr), { error: 'INVALID_PREPARATION_INPUT' }); return error.code === 1;
  });
});
