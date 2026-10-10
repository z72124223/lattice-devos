import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { lifeHarnessCommand, parseLifeHarnessArguments, runLifeHarnessClient } from '../src/life-harness-client.mjs';

const args = action => [action, '--origin', 'http://127.0.0.1:4317', '--project', 'project', '--task', 'a'.repeat(64),
  '--claim', 'claim', '--thread', 'thread', '--turn', 'turn', '--failure', 'failure'];
const binding = { projectId: 'project', taskRef: 'a'.repeat(64), claimId: 'claim', threadId: 'thread',
  turnId: 'turn', inputId: 'input', failureItemId: 'failure' };
const hash = value => createHash('sha256').update(value).digest('hex');
async function fixture(t) {
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'lattice-client-test-')));
  const checked = path.resolve(root);
  t.after(async () => {
    assert.equal(path.resolve(await realpath(root)), checked);
    assert.ok(path.basename(checked).startsWith('lattice-client-test-'));
    await rm(root, { recursive: true });
  });
  const importer = path.join(root, 'entry.mjs');
  const importerBytes = "import './missing.mjs';\nthrow new Error('This module must not execute during preparation');\n";
  await writeFile(importer, importerBytes);
  const event = { id: 'failure', type: 'commandExecution', status: 'failed', exitCode: 1,
    command: `node '${importer}'`,
    aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(root, 'missing.mjs')}' imported from ${importer}\n` };
  const failureJson = JSON.stringify(event);
  const source = { schema: 'lattice.control.diagnostic-source.v1', binding, projectRoot: root,
    nativeSourceVerified: true, contextBasis: 'existing_owned_formal_execution', advisoryOnly: true,
    adopted: false, grantsNewAuthority: false, repairAuthorized: false, failureJson, failureSha256: hash(failureJson),
    context: { projectId: 'project', taskId: binding.taskRef, authority: 'authorized', projectScope: 'matched',
      lifecycle: 'active', freshness: 'current', circuitOpen: false, validUntil: new Date(Date.now() + 120000).toISOString() } };
  return { root, source, importer, importerBytes, fetchImpl: async (url, options) => {
    const parsed = new URL(url);
    assert.equal(parsed.searchParams.get('failureItemId'), 'failure');
    assert.equal(options.redirect, 'error'); assert.ok(options.signal);
    return Response.json(source);
  } };
}

test('prepare fetches the selected owned source, preserves its bytes and only returns a separate receipt command', async t => {
  const f = await fixture(t);
  const result = await runLifeHarnessClient(args('prepare'), f);
  const archive = path.dirname(result.sourceFile);
  t.after(async () => {
    assert.equal(path.dirname(await realpath(archive)), await realpath(tmpdir()));
    assert.ok(path.basename(archive).startsWith('lattice-diagnostic-'));
    await rm(archive, { recursive: true });
  });
  assert.equal(result.decision, 'prepared');
  assert.match(result.command, /relative-module-diagnostic-cli\.mjs.*--receipt [a-f0-9]{64}$/u);
  assert.match(result.readbackCommand, /life-harness-client\.mjs' readback/u);
  assert.deepEqual(result.binding, binding);
  assert.equal(result.trust.nativeProvenanceVerified, false);
  assert.equal(result.trust.authorizationVerified, false);
  assert.equal(await readFile(f.importer, 'utf8'), f.importerBytes);
  const saved = JSON.parse(await readFile(result.sourceFile, 'utf8'));
  assert.equal(saved.failureJson, f.source.failureJson);
  assert.equal(await readFile(path.join(path.dirname(result.sourceFile), 'failure.json'), 'utf8'), f.source.failureJson);
  const request = JSON.parse(await readFile(result.artifacts.requestPath, 'utf8'));
  assert.deepEqual(request.receiptBinding, binding);
  assert.equal(request.context.validUntil, f.source.context.validUntil);
  assert.equal((await readdir(f.root)).includes('missing.mjs'), false);
  assert.deepEqual(await readdir(f.root), ['entry.mjs']);
});

test('changed identity, source bytes, authority, expiry and outside importer are rejected before writing', async t => {
  for (const [name, change] of [
    ['binding', s => { s.binding = { ...binding, turnId: 'old' }; }],
    ['hash', s => { s.failureJson += ' '; }],
    ['authority', s => { s.grantsNewAuthority = true; }],
    ['expired', s => { s.context.validUntil = new Date(0).toISOString(); }],
    ['outside importer', s => { const e = JSON.parse(s.failureJson); e.aggregatedOutput = `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.resolve('missing.mjs')}' imported from ${path.resolve('outside.mjs')}\n`; s.failureJson = JSON.stringify(e); s.failureSha256 = hash(s.failureJson); }],
  ]) await t.test(name, async t => {
    const f = await fixture(t); change(f.source);
    await assert.rejects(runLifeHarnessClient(args('prepare'), f));
    assert.deepEqual(await readdir(f.root), ['entry.mjs']);
  });
});

test('HTTP ambiguity and errors are preserved without retry or filesystem writes', async t => {
  const f = await fixture(t); let calls = 0;
  await assert.rejects(runLifeHarnessClient(args('prepare'), { fetchImpl: async () => {
    calls++; return Response.json({ code: 'CONTROL_DIAGNOSTIC_SOURCE_AMBIGUOUS_FAILURE' }, { status: 409 });
  } }), { code: 'CONTROL_DIAGNOSTIC_SOURCE_AMBIGUOUS_FAILURE' });
  assert.equal(calls, 1); assert.deepEqual(await readdir(f.root), ['entry.mjs']);
});

test('readback retains limited trust and refuses mismatched binding or elevated assertions', async () => {
  const value = { schema: 'lattice.control.relative-module-diagnostic.v1', binding,
    nativeClaimBindingVerified: true, producerVerified: false, inputFileBytesVerified: false,
    diagnosticSemanticsVerified: false, receipt: { binding, validUntil: new Date(Date.now() + 120000).toISOString(),
      result: { advisoryOnly: true, adopted: false, trust: { nativeProvenanceVerified: false, authorizationVerified: false } } } };
  assert.deepEqual(await runLifeHarnessClient(args('readback'), { fetchImpl: async () => Response.json(value) }), value);
  for (const mutate of [v => { v.producerVerified = true; },
    v => { v.receipt.validUntil = new Date(0).toISOString(); },
    v => { v.receipt.validUntil = new Date(Date.now() + 600000).toISOString(); },
    v => { v.receipt.binding.inputId = 'other'; },
    v => { v.receipt.result.trust.nativeProvenanceVerified = true; },
    v => { v.receipt.result.trust.authorizationVerified = true; },
  ]) {
    const changed = JSON.parse(JSON.stringify(value)); mutate(changed);
    await assert.rejects(runLifeHarnessClient(args('readback'), { fetchImpl: async () => Response.json(changed) }), { code: 'LIFE_HARNESS_READBACK_REJECTED' });
  }
});

test('arguments reject remote hosts, credentials, duplicate selectors and unsafe command literals', () => {
  for (const origin of ['https://example.com', 'http://user:pass@127.0.0.1', 'http://localhost:4317']) {
    const input = args('prepare'); input[2] = origin;
    assert.throws(() => parseLifeHarnessArguments(input));
  }
  assert.throws(() => parseLifeHarnessArguments([...args('prepare'), '--turn', 'other']));
  assert.throws(() => lifeHarnessCommand({ ...parseLifeHarnessArguments(args('prepare')), turn: "x'; echo bad" }));
});

test('unbounded response is rejected', async () => {
  await assert.rejects(runLifeHarnessClient(args('prepare'), { fetchImpl: async () => new Response('x'.repeat(2 * 1024 * 1024 + 1)) }),
    { code: 'LIFE_HARNESS_RESPONSE_TOO_LARGE' });
});
