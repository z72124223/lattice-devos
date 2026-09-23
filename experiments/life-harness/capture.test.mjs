import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { selectNative, buildControlledCases, verifyLocalCapture, verifyRawCapture, reviewedCapture, scenarios } from './capture.mjs';
import { validateCase, validateSources, advise, hash } from './harness.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const captured = fs.readFileSync(path.join(here, 'native-fixtures/captured-events.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
const cases = fs.readFileSync(path.join(here, 'controlled-cases.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
const fixture = path.join(here, 'native-fixtures');

test('controlled projection retains native statuses and codes; does not relabel failures from exit codes', () => {
  assert.deepEqual(buildControlledCases(), cases);
  cases.forEach(validateCase); validateSources(cases);
  for (let i = 0; i < cases.length; i++) {
    assert.equal(cases[i].event.status, captured[i].record.payload.item.status);
    assert.equal(cases[i].event.exitCode, captured[i].record.payload.item.exit_code);
    assert.equal(cases[i].source.kind, 'controlled_replay'); assert.equal(cases[i].source.native_observed, true);
  }
  assert.deepEqual(cases.map(c => [c.event.status, c.event.exitCode]), [['failed', 1], ['failed', 1], ['completed', 0]]);
});

// Synthetic parser tests only. These are never added to the captured corpus.
function parserFixture() {
  return [ { type: 'session_meta', payload: { id: 'own-task' } },
    { type: 'event_msg', payload: { type: 'task_started', turn_id: 'own-turn' } },
    { type: 'turn_context', payload: { turn_id: 'own-turn' } },
    ...captured.map(c => { const r = structuredClone(c.record); r.payload.thread_id = 'own-task'; r.payload.turn_id = 'own-turn';
      r.payload.item.cwd = pathToFileURL(fixture).href; return r; }) ];
}
const select = rows => selectNative(rows.map(JSON.stringify), 'own-task', 'own-turn', fixture);

test('collector rejects wrong task/turn/cwd, missing context, duplicated event and already terminal turn', () => {
  const rows = parserFixture(); assert.equal(select(rows).selected.length, 3);
  for (const mutate of [
    r => { r[0].payload.id = 'foreign'; }, r => { r[3].payload.turn_id = 'foreign'; },
    r => { r[3].payload.item.cwd = pathToFileURL(here).href; }, r => { r.splice(2, 1); },
    r => { r.push(r[3]); }, r => { r.splice(3, 0, { type: 'event_msg', payload: { type: 'task_complete', turn_id: 'own-turn' } }); },
    r => { r[3].payload.item.type = 'function_call_output'; },
  ]) { const changed = structuredClone(rows); mutate(changed); assert.throws(() => select(changed)); }
});

test('a platform completed/nonzero event remains completed and is excluded by the existing contract', () => {
  const rows = parserFixture(); rows[3].payload.item.status = 'completed';
  const selected = select(rows).selected[0].record.payload.item;
  assert.equal(selected.status, 'completed'); assert.equal(selected.exit_code, 1);
  const c = structuredClone(cases[0]); c.event.status = selected.status;
  c.source.evidence_sha256 = hash(JSON.stringify({ event: c.event, context: c.context }));
  assert.equal(advise(c, 'A', { dataset_sha256: 'a'.repeat(64), procedures_sha256: 'b'.repeat(64), implementation_sha256: 'c'.repeat(64) }).decision, 'excluded');
});

test('portable raw absence is explicit; local original is checked when available', () => {
  assert.deepEqual(verifyLocalCapture(path.join(here, 'absent-raw-capture.jsonl')),
    { portable_projection: 'verified', native_original: 'not_available_locally' });
  assert.ok(['not_available_locally', 'verified_from_local_raw_capture'].includes(verifyLocalCapture().native_original));
});

test('reviewed capture binding rejects edited status, kind and raw contents', () => {
  const content = fs.readFileSync(path.join(here, 'native-fixtures/captured-events.jsonl'), 'utf8');
  reviewedCapture(content);
  assert.throws(() => reviewedCapture(content.replace('"failed"', '"completed"')));
  assert.throws(() => reviewedCapture(content.replace('"controlled_replay"', '"real"')));
  assert.throws(() => verifyRawCapture('{}\n'));
});

test('scope contains no formal identity, and the three small scenario commands are fixed', () => {
  assert.equal(scenarios.length, 3);
  for (const c of captured) {
    assert.equal(c.scope.formal_project, null); assert.equal(c.scope.formal_task, null);
    assert.ok(!/C:[\\/]|f7212|01a0ce00/iu.test(JSON.stringify(c)));
  }
});
