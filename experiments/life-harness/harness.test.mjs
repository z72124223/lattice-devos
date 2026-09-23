import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { here, hash, jsonl, validateCase, validateAdvisory, validateSources, validateAnnotations, validateAdjudication,
  advise, evaluate, exclusion } from './harness.mjs';
import { resolveGitExecutable } from '../../apps/lattice-control/src/project-inspector.mjs';
import { resolveWindowsCodexRuntime } from '../../apps/lattice-control/src/codex-runtime-resolution.mjs';

const cases = jsonl(path.join(here, 'cases.jsonl'));
const binding = { dataset_sha256: hash(fs.readFileSync(path.join(here, 'cases.jsonl'))),
  procedures_sha256: 'a'.repeat(64), implementation_sha256: 'b'.repeat(64) };
const get = suffix => structuredClone(cases.find(c => c.case_id === `offline-synthetic-${suffix}`));
function reseal(c) { c.source.evidence_sha256 = hash(JSON.stringify({ event: c.event, context: c.context })); return c; }

test('source hashes and native projection are verified; arbitrary source paths and fabricated refs fail', () => {
  cases.forEach(validateCase); validateSources(cases);
  const c = structuredClone(cases[0]); c.event.status = 'failed'; reseal(c);
  assert.throws(() => validateSources([c]));
  const fake = get('git-missing'); fake.formal_refs.task = 'invented-task';
  assert.throws(() => validateSources([fake]));
  fake.formal_refs.task = null; fake.source.reference = '../../private';
  assert.throws(() => validateSources([fake]));
  for (const patch of [{ revision: 'c'.repeat(40) }, { selector: 'definitions/999' }, { selector: 'definitions/1' }]) {
    const changed = get('git-missing'); Object.assign(changed.source, patch);
    assert.throws(() => validateSources([changed]));
  }
});

test('strict schema rejects extra instructions, wrong types, oversize output and evidence mutation', () => {
  for (const change of [
    c => { c.instruction = 'execute'; }, c => { c.context.circuit_open = 'false'; },
    c => { c.event.exitCode = '1'; }, c => { c.event.aggregatedOutput = 'x'.repeat(4097); reseal(c); },
    c => { c.event.aggregatedOutput = 'different'; }, c => { c.source.native_observed = true; },
  ]) { const c = get('git-missing'); change(c); assert.throws(() => validateCase(c)); }
});

test('denied, terminal, expired, identity and circuit boundaries stop before retrieval in both groups', () => {
  const blocked = cases.filter(c => exclusion(c));
  assert.equal(blocked.length, 13);
  for (const c of blocked) for (const group of ['A', 'B']) {
    const a = advise(c, group, binding, () => { throw new Error('retrieval reached a boundary'); });
    assert.equal(a.decision, 'excluded'); assert.deepEqual(a.candidates, []);
  }
  for (const patch of [{ authority: 'denied' }, { freshness: 'unknown' }, { lifecycle: 'unknown' }]) {
    const c = get('git-missing'); Object.assign(c.context, patch); reseal(c);
    assert.equal(advise(c, 'A', binding).decision, 'excluded');
  }
});

test('synthetic native-shaped items cannot be promoted to a native observed case', () => {
  const c = get('git-missing'); c.source.kind = 'real';
  assert.equal(advise(c, 'A', binding).decision, 'excluded');
  c.source.native_observed = true;
  assert.throws(() => validateSources([c]));
});

test('malicious log text is data and only a fixed procedure ID can be selected', () => {
  const a = advise(get('log-instruction'), 'A', binding);
  assert.equal(a.selected_procedure, 'git-resolution');
  assert.equal(a.advisory_only, true);
  assert.ok(!JSON.stringify(a).includes('example.invalid'));
  const bad = { ...a, selected_procedure: 'run-upload' };
  assert.throws(() => validateAdvisory(bad));
  assert.throws(() => validateAdvisory({ ...a, command: 'shell' }));
});

test('misleading package keyword and insufficient evidence do not force a candidate', () => {
  assert.equal(advise(get('misleading-keyword'), 'A', binding).abstention_reason, 'none_applicable');
  assert.equal(advise(get('insufficient'), 'A', binding).abstention_reason, 'insufficient_information');
  for (const output of ['', null]) {
    const c = get('insufficient'); c.event.aggregatedOutput = output; reseal(c);
    assert.equal(advise(c, 'A', binding).abstention_reason, 'insufficient_information');
  }
});

test('retrieval misses, ranking mistakes and abstentions have distinct denominators', () => {
  const c = get('multiple-valid');
  const g = { case_id: c.case_id, label: 'valid', valid_procedures: ['git-resolution'] };
  const a = advise(c, 'A', binding);
  const score = patch => evaluate([c], [g], [{ ...a, ...patch }]).synthetic;
  assert.equal(score({ candidates: ['codex-resolution'], selected_procedure: 'codex-resolution' }).retrieval_misses, 1);
  assert.equal(score({ candidates: ['codex-resolution'], selected_procedure: 'codex-resolution' }).ranking_errors, 0);
  assert.equal(score({ selected_procedure: 'codex-resolution' }).ranking_errors, 1);
  assert.equal(score({ decision: 'abstain', selected_procedure: null }).abstentions_with_recall, 1);
  assert.equal(evaluate([], [], []).real.retrieval_recall, null);
  const multi = { ...g, valid_procedures: ['git-resolution', 'codex-resolution'] };
  assert.equal(evaluate([c], [multi], [{ ...a, selected_procedure: 'codex-resolution' }]).synthetic.selection_hits, 1);
});

test('frozen B differs from A only in group and measured local evaluator timing', () => {
  for (const c of cases) {
    const { group: ga, elapsed_ms: ta, ...a } = advise(c, 'A', binding);
    const { group: gb, elapsed_ms: tb, ...b } = advise(c, 'B', binding);
    assert.deepEqual(a, b);
  }
});

test('annotation binding rejects dataset/procedure drift and duplicated/missing case IDs', () => {
  const rows = cases.map(c => ({ case_id: c.case_id, dataset_sha256: binding.dataset_sha256,
    procedures_sha256: binding.procedures_sha256,
    annotator: 'test', label: 'excluded', valid_procedures: [], reason: 'schema-only fixture' }));
  validateAnnotations(rows, cases, binding.dataset_sha256, binding.procedures_sha256, 'test');
  assert.throws(() => validateAnnotations(rows, cases, 'c'.repeat(64), binding.procedures_sha256, 'test'));
  assert.throws(() => validateAnnotations(rows, cases, binding.dataset_sha256, 'c'.repeat(64), 'test'));
  assert.throws(() => validateAnnotations([...rows.slice(1), rows[1]], cases, binding.dataset_sha256, binding.procedures_sha256, 'test'));
});

test('adjudication rejects concealed disagreements and unrecorded gold overrides', () => {
  const row = { case_id: 'offline-synthetic-git-missing', label: 'valid', valid_procedures: ['git-resolution'] };
  const record = { schema_version: 1, dataset_sha256: binding.dataset_sha256, procedures_sha256: binding.procedures_sha256,
    reviewed_cases: 1, annotation_completion_is_product_acceptance: false,
    roles: { alpha: 'alpha', beta: 'beta', adjudicator: 'root' }, disagreements: [] };
  validateAdjudication(record, [row], [row], [row], binding);
  const other = { ...row, label: 'none_applicable', valid_procedures: [] };
  assert.throws(() => validateAdjudication(record, [row], [other], [row], binding));
  assert.throws(() => validateAdjudication(record, [row], [row], [other], binding));
});

test('controlled local replay exercises existing diagnostic failures without native event fabrication', async () => {
  await assert.rejects(resolveGitExecutable({ cwd: path.resolve(here, '../..'), pathValue: '' }),
    error => error.code === 'GIT_EXECUTABLE_UNAVAILABLE');
  await assert.rejects(resolveWindowsCodexRuntime({ env: {}, probeVersion: () => { throw new Error('must not spawn'); } }),
    /Codex runtime was not found/u);
  // These are actual local function failures, not native commandExecution results.
  // No case or production-effect receipt is written from this test.
});
