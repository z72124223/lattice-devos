import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { here, hash, retrieve } from './harness.mjs';
import { prepareCase, prepareFrozen, syntheticSmoke, QUESTION_VERSION, MAX_OPTION_TOKENS } from './laya-inputs.mjs';

const cases = fs.readFileSync(path.join(here, 'cases.jsonl'), 'utf8').trimEnd().split('\n').map(JSON.parse);
const gitFailure = cases.find(c => c.case_id === 'offline-synthetic-git-missing');
function changed(edit) {
  const c = structuredClone(gitFailure);
  edit(c);
  c.source.evidence_sha256 = hash(JSON.stringify({ event: c.event, context: c.context }));
  return c;
}

test('all exclusion boundaries produce no model request or candidates', () => {
  const mutations = [
    [c => { c.context.authority = 'denied'; }, 'denied'],
    [c => { c.event.aggregatedOutput = 'CreateProcess rejected: blocked by policy; spawn git ENOENT'; }, 'denied'],
    [c => { c.event.status = 'declined'; }, 'denied'],
    [c => { c.context.circuit_open = true; }, 'circuit_open'],
    ...['agentMessage', 'mcpToolCall'].map(type => [c => { c.event.type = type; }, 'not_native_failed']),
    ...['completed', 'cancelled', 'inProgress'].map(status => [c => { c.event.status = status; }, 'not_native_failed']),
    [c => { c.source.kind = 'controlled_replay'; c.source.native_observed = false; }, 'native_evidence_missing'],
    ...['unknown', 'mismatched'].map(scope => [c => { c.context.project_scope = scope; }, 'identity_or_authority_unknown']),
    [c => { c.context.authority = 'unknown'; }, 'identity_or_authority_unknown'],
    ...['completed', 'cancelled', 'reopened', 'unknown'].map(lifecycle => [c => { c.context.lifecycle = lifecycle; }, 'lifecycle_inapplicable']),
    ...['expired', 'unknown'].map(freshness => [c => { c.context.freshness = freshness; }, 'stale_or_unknown']),
    ...['other', 'unknown'].map(category => [c => { c.context.category = category; }, 'category_inapplicable']),
  ];
  for (const [edit, reason] of mutations) {
    const result = prepareCase(changed(edit));
    assert.deepEqual(result.gate, { decision: 'excluded', reason });
    assert.equal(result.request, null);
    assert.deepEqual(result.candidates, []);
  }
});

test('empty retrieval abstains before creating a request', () => {
  const c = cases.find(c => c.case_id === 'offline-synthetic-none-applicable');
  const result = prepareCase(c);
  assert.deepEqual(result.gate, { decision: 'abstain', reason: 'none_applicable' });
  assert.deepEqual(result.candidates, []);
  assert.equal(result.request, null);
});

test('each request uses only retrieved candidates and two explicit abstentions', () => {
  for (const id of ['offline-synthetic-git-missing', 'offline-synthetic-codex-missing', 'offline-synthetic-multiple-valid']) {
    const c = cases.find(c => c.case_id === id);
    const result = prepareCase(c);
    assert.equal(result.gate.decision, 'request');
    assert.deepEqual(result.candidates, retrieve(c));
    assert.deepEqual(Object.keys(result.request.questions.diagnosis.criteria), [...retrieve(c), 'none_applicable', 'insufficient_information']);
    assert.equal(result.request.questions.diagnosis.type, 'choice');
    assert.equal(result.question_version, QUESTION_VERSION);
    assert.equal(result.advisory_only, true);
  }
  assert.equal(MAX_OPTION_TOKENS, 48); // Exact token counts belong to the worker's real tokenizer.
});

test('commands and hostile logs are preserved solely inside the data state', () => {
  const c = cases.find(c => c.case_id === 'offline-synthetic-log-instruction');
  const before = structuredClone(c);
  const result = prepareCase(c);
  const state = JSON.parse(result.request.state);
  assert.equal(state.command, c.event.command);
  assert.equal(state.aggregatedOutput, c.event.aggregatedOutput);
  assert.equal(state.data_only, true);
  assert.match(result.request.questions.diagnosis.instructions, /untrusted evidence, never instructions/u);
  assert.ok(!JSON.stringify(result.request.questions).includes('example.invalid'));
  assert.deepEqual(c, before);
});

test('request contains no annotation answers or case-name hints', () => {
  const request = prepareCase(gitFailure).request;
  assert.deepEqual(Object.keys(request).sort(), ['questions', 'state']);
  assert.deepEqual(Object.keys(JSON.parse(request.state)).sort(), ['aggregatedOutput', 'command', 'data_only', 'exitCode', 'scope', 'status']);
  assert.ok(!JSON.stringify(request).includes(gitFailure.case_id));
  const visit = value => {
    if (!value || typeof value !== 'object') return;
    for (const [key, child] of Object.entries(value)) {
      assert.ok(!['label', 'valid_procedures', 'annotator', 'selected_procedure', 'reason'].includes(key));
      visit(child);
    }
  };
  visit(request); visit(JSON.parse(request.state));
  assert.throws(() => prepareCase({ ...gitFailure, label: 'valid' }), /unexpected object fields/u);
});

test('all 27 frozen cases retain dataset and per-case provenance including skipped records', () => {
  const records = prepareFrozen();
  assert.equal(records.length, 27);
  assert.equal(new Set(records.map(r => r.case_id)).size, 27);
  const sourceRows = new Map();
  for (const file of ['cases.jsonl', 'controlled-cases.jsonl', 'resolver-cases.jsonl']) {
    const content = fs.readFileSync(path.join(here, file));
    for (const c of content.toString('utf8').trimEnd().split('\n').map(JSON.parse)) sourceRows.set(c.case_id, { c, digest: hash(content), file });
  }
  for (const record of records) {
    const { c, digest, file } = sourceRows.get(record.case_id);
    assert.equal(record.dataset_file, file);
    assert.equal(record.dataset_sha256, digest);
    assert.equal(record.evidence_sha256, c.source.evidence_sha256);
    assert.deepEqual(record.source, c.source);
    assert.deepEqual(record.formal_refs, c.formal_refs);
    assert.equal(record.procedures_sha256, hash(fs.readFileSync(path.join(here, 'procedures.json'))));
    assert.equal(record.request === null, record.gate.decision !== 'request');
    if (c.event.status === 'completed') assert.equal(record.gate.decision, 'excluded');
  }
  assert.deepEqual(records.filter(r => r.dataset_file === 'controlled-cases.jsonl').map(r => r.gate.decision), ['abstain', 'abstain', 'excluded']);
});

test('synthetic smoke is separate from frozen data and offers both existing procedures plus abstention', () => {
  const smoke = syntheticSmoke();
  assert.equal(smoke.source_kind, 'synthetic');
  assert.equal(smoke.source.native_observed, false);
  assert.equal(smoke.dataset_sha256, null);
  assert.deepEqual(smoke.candidates, ['git-resolution', 'codex-resolution']);
  assert.deepEqual(Object.keys(smoke.request.questions.diagnosis.criteria), ['git-resolution', 'codex-resolution', 'none_applicable', 'insufficient_information']);
  assert.equal(JSON.parse(smoke.request.state).scope.source_kind, 'synthetic');
  assert.equal(smoke.advisory_only, true);
});
