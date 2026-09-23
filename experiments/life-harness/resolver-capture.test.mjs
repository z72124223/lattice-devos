import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { buildControlledCases, reviewedCapture, verifyLocalCapture, verifyRawCapture } from './capture.mjs';
import { validateCase, validateSources } from './harness.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const options = { resolvers: true };
const projection = fs.readFileSync(path.join(here, 'resolver-fixtures/captured-events.jsonl'), 'utf8');
const cases = fs.readFileSync(path.join(here, 'resolver-cases.jsonl'), 'utf8').trimEnd().split('\n').map(JSON.parse);

test('resolver cases preserve the four native statuses and remain a separate controlled slice', () => {
  assert.deepEqual(buildControlledCases(options), cases);
  cases.forEach(validateCase); validateSources(cases);
  assert.deepEqual(cases.map(c => [c.event.status, c.event.exitCode]), [['failed', 1], ['failed', 1], ['completed', 0], ['completed', 0]]);
  for (const c of cases) {
    assert.equal(c.source.kind, 'controlled_replay');
    assert.equal(c.source.native_observed, true);
    assert.equal(c.group_id, 'controlled-resolvers-capture-20260923');
    assert.deepEqual(c.formal_refs, { project: null, task: null });
    assert.ok(!/[A-Za-z]:[\\/]|f7212|01a0ce00/u.test(JSON.stringify(c)));
  }
});

test('resolver source binding rejects cross-slice data, edited status and fabricated raw capture', () => {
  reviewedCapture(projection, options);
  assert.throws(() => reviewedCapture(projection));
  assert.throws(() => reviewedCapture(projection.replace('"failed"', '"completed"'), options));
  assert.throws(() => verifyRawCapture('{}\n', options));
  const altered = structuredClone(cases[0]);
  altered.source.reference = 'experiments/life-harness/native-fixtures/captured-events.jsonl';
  assert.throws(() => validateSources([altered]));
  assert.deepEqual(verifyLocalCapture(path.join(here, 'absent-resolver-raw.jsonl'), options),
    { portable_projection: 'verified', native_original: 'not_available_locally' });
  assert.ok(['not_available_locally', 'verified_from_local_raw_capture'].includes(verifyLocalCapture(undefined, options).native_original));
});

test('resolver collection refuses any partial archive before writes in an isolated checkout', async t => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'life-harness-resolver-'));
  t.after(() => {
    assert.equal(path.dirname(path.resolve(tempRoot)), path.resolve(os.tmpdir()));
    assert.ok(path.basename(tempRoot).startsWith('life-harness-resolver-'));
    fs.rmSync(tempRoot, { recursive: true, force: true });
  });
  const outputs = ['experiments/life-harness/resolver-fixtures/captured-events.jsonl',
    '.lattice/life-harness/native-resolvers-20260923/raw-capture.jsonl',
    '.lattice/life-harness/native-resolvers-20260923/source-index.json',
    'experiments/life-harness/resolver-cases.jsonl'];
  for (const [index, output] of outputs.entries()) {
    const isolatedRoot = path.join(tempRoot, String(index));
    const isolatedHere = path.join(isolatedRoot, 'experiments/life-harness');
    fs.mkdirSync(isolatedHere, { recursive: true });
    fs.copyFileSync(path.join(here, 'capture.mjs'), path.join(isolatedHere, 'capture.mjs'));
    const existing = path.join(isolatedRoot, output);
    fs.mkdirSync(path.dirname(existing), { recursive: true });
    fs.writeFileSync(existing, 'frozen archive sentinel\n');
    const before = fs.readdirSync(isolatedRoot, { recursive: true }).sort();
    const { capture } = await import(pathToFileURL(path.join(isolatedHere, 'capture.mjs')).href);
    assert.throws(() => capture(path.join(isolatedRoot, 'absent-own-task.jsonl'), 'own-task', 'own-turn', options),
      /capture archive already exists:/u);
    assert.deepEqual(fs.readdirSync(isolatedRoot, { recursive: true }).sort(), before);
    assert.equal(fs.readFileSync(existing, 'utf8'), 'frozen archive sentinel\n');
    for (const absent of outputs.filter(file => file !== output)) assert.equal(fs.existsSync(path.join(isolatedRoot, absent)), false);
  }
});
